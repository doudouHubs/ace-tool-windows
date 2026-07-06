use crate::index::{LocalIndexRebuildMode, LocalRerankMode, LocalSummaryMode};
use crate::index::manager::{
    is_binary_content, normalize_path, sanitize_content, should_exclude_path,
};
use crate::logging::{log_debug, log_debug_verbose};
use crate::utils::encoding::read_file_with_encoding;
use crate::utils::ignore::load_gitignore;
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

const INDEX_VERSION: u32 = 3;
const DEFAULT_CHUNK_LINES: usize = 120;
const DEFAULT_CHUNK_OVERLAP_LINES: usize = 20;
const DEFAULT_TOP_K: usize = 8;
const DEFAULT_SUMMARY_TIMEOUT_SEC: u64 = 90;
const DEFAULT_SUMMARY_SOFT_TIMEOUT_SEC: u64 = 60;
const MIN_SUMMARY_BUDGET_SEC: u64 = 8;
const SUMMARY_RESPONSE_GUARD_SEC: u64 = 5;
const SUMMARY_MAX_CANDIDATES: usize = 5;
const SUMMARY_SNIPPET_MAX_LINES: usize = 18;
const SUMMARY_SNIPPET_MAX_CHARS: usize = 1800;
const SUMMARY_CONTEXT_MAX_CHARS: usize = 18000;
const LOCAL_SEARCH_DIR: &str = "local-search";
const META_FILE: &str = "meta.json";
const MANIFEST_FILE: &str = "files-manifest.json";
const LEGACY_CHUNKS_FILE: &str = "chunks.json";
const CHUNKS_DIR: &str = "chunks";
const QUERY_CACHE_DIR: &str = "query-cache";
const RERANK_CACHE_DIR: &str = "rerank-cache";
const QUERY_CACHE_TTL_SEC: u64 = 7 * 24 * 60 * 60;
const RERANK_CACHE_TTL_SEC: u64 = 3 * 24 * 60 * 60;
const QUERY_CACHE_MAX_ENTRIES: usize = 200;
const RERANK_CACHE_MAX_ENTRIES: usize = 200;
const SOFT_MAX_RESULTS_PER_FILE: usize = 2;
const RANK_CANDIDATE_POOL_MULTIPLIER: usize = 4;
const MIN_RERANK_CANDIDATES: usize = 4;
const DEFAULT_RERANK_TIMEOUT_SEC: u64 = 10;
const RERANK_RESPONSE_GUARD_SEC: u64 = 6;
const MIN_RERANK_BUDGET_SEC: u64 = 4;
const RERANK_CONTEXT_MAX_CHARS: usize = 9000;

#[derive(Clone)]
pub struct LocalSearchProvider {
    project_root: PathBuf,
    storage_dir: PathBuf,
    chunks_dir: PathBuf,
    query_cache_dir: PathBuf,
    rerank_cache_dir: PathBuf,
    text_extensions: HashSet<String>,
    exclude_patterns: Vec<String>,
    max_lines_per_blob: usize,
    codex_api_base: String,
    codex_model: String,
    rerank_model: String,
    summary_mode: LocalSummaryMode,
    rerank_mode: LocalRerankMode,
    index_rebuild_mode: LocalIndexRebuildMode,
    search_timeout_sec: u64,
    rerank_pool_size: usize,
    rerank_timeout_sec: u64,
    client: Option<Client>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LocalChunkRecord {
    id: String,
    path: String,
    file_name: String,
    start_line: usize,
    end_line: usize,
    content_hash: String,
    content: String,
    normalized_text: String,
    token_freq: HashMap<String, u32>,
    token_count: usize,
    symbol_tokens: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LocalIndexMeta {
    version: u32,
    project_root: String,
    retrieval_mode: String,
    summary_mode: String,
    summary_model: String,
    index_mode: String,
    chunk_lines: usize,
    chunk_overlap_lines: usize,
    indexed_at_unix_sec: u64,
    file_count: usize,
    chunk_count: usize,
    index_signature: String,
    #[serde(default)]
    rebuild_reason: String,
    #[serde(default)]
    storage_health: String,
    #[serde(default)]
    orphan_chunk_files_deleted: usize,
    #[serde(default)]
    query_cache_entries: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct LocalFileManifest {
    version: u32,
    files: Vec<LocalFileManifestEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LocalFileManifestEntry {
    path: String,
    file_name: String,
    modified_unix_sec: u64,
    size: u64,
    file_hash: String,
    chunk_file: String,
    chunk_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct QueryCacheEntry {
    query: String,
    candidate_signature: String,
    summary_mode: String,
    result: String,
    cached_at_unix_sec: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RerankCacheEntry {
    query: String,
    candidate_signature: String,
    rerank_model: String,
    rerank_mode: String,
    ordered_candidate_ids: Vec<String>,
    cached_at_unix_sec: u64,
}

#[derive(Clone, Debug)]
struct DiscoveredFile {
    absolute_path: PathBuf,
    path: String,
    file_name: String,
    modified_unix_sec: u64,
    size: u64,
}

#[derive(Clone, Debug)]
struct IndexUpdateStats {
    index_mode: String,
    rebuild_reason: String,
    storage_health: String,
    reused_files: usize,
    updated_files: usize,
    deleted_files: usize,
    orphan_chunk_files_deleted: usize,
    file_count: usize,
    chunk_count: usize,
    index_signature: String,
    query_cache_entries: usize,
}

#[derive(Clone, Debug)]
struct StorageHealthStatus {
    requires_full_rebuild: bool,
    rebuild_reason: Option<String>,
    health_label: String,
}

impl StorageHealthStatus {
    fn healthy(label: &str) -> Self {
        Self {
            requires_full_rebuild: false,
            rebuild_reason: None,
            health_label: label.to_string(),
        }
    }

    fn requires_rebuild(reason: &str) -> Self {
        Self {
            requires_full_rebuild: true,
            rebuild_reason: Some(reason.to_string()),
            health_label: "recovered".to_string(),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct QueryCachePruneStats {
    kept_entries: usize,
    expired_removed: usize,
    invalid_removed: usize,
    overflow_removed: usize,
}

impl QueryCachePruneStats {
    fn total_removed(&self) -> usize {
        self.expired_removed + self.invalid_removed + self.overflow_removed
    }
}

#[derive(Clone, Debug)]
struct QueryCandidate {
    score: f32,
    chunk: LocalChunkRecord,
    matched_terms: Vec<String>,
    matched_phrases: Vec<String>,
    reason_labels: Vec<String>,
}

#[derive(Clone, Debug)]
struct QueryTerms {
    weighted_terms: Vec<WeightedTerm>,
    phrases: Vec<WeightedPhrase>,
}

#[derive(Clone, Debug)]
struct WeightedTerm {
    term: String,
    weight: f32,
}

#[derive(Clone, Debug)]
struct WeightedPhrase {
    phrase: String,
    weight: f32,
}

#[derive(Clone, Debug)]
struct QueryMatchOutcome {
    score: f32,
    matched_terms: Vec<String>,
    matched_phrases: Vec<String>,
    reason_labels: Vec<String>,
}

#[derive(Clone, Debug)]
struct PreparedQuery {
    terms: QueryTerms,
    broad_intent: bool,
    hint_terms: Vec<String>,
    hint_phrases: Vec<String>,
}

#[derive(Clone, Debug)]
struct RerankCandidate {
    id: String,
    path: String,
    start_line: usize,
    end_line: usize,
    reason_summary: String,
    snippet: String,
}

impl LocalSearchProvider {
    pub fn new(
        project_root: PathBuf,
        text_extensions: HashSet<String>,
        max_lines_per_blob: usize,
        exclude_patterns: Vec<String>,
        codex_api_base: String,
        codex_api_key: String,
        codex_model: String,
        summary_mode: LocalSummaryMode,
        rerank_mode: LocalRerankMode,
        index_rebuild_mode: LocalIndexRebuildMode,
        search_timeout_sec: u64,
        rerank_pool_size: usize,
        rerank_timeout_sec: u64,
        rerank_model: String,
    ) -> Result<Self, String> {
        let needs_client =
            summary_mode == LocalSummaryMode::Gpt || rerank_mode != LocalRerankMode::Off;
        let client = if needs_client {
            if codex_api_base.trim().is_empty() {
                return Err(
                    "Local search requires --codex-api-base or ACE_TOOL_CODEX_API_BASE."
                        .to_string(),
                );
            }
            if codex_api_key.trim().is_empty() {
                return Err(
                    "Local search requires --codex-api-key or ACE_TOOL_CODEX_API_KEY."
                        .to_string(),
                );
            }

            let mut headers = HeaderMap::new();
            let auth_header = format!("Bearer {}", codex_api_key.trim());
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&auth_header).map_err(|e| e.to_string())?,
            );
            headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

            Some(
                Client::builder()
                    .default_headers(headers)
                    .timeout(Duration::from_secs(DEFAULT_SUMMARY_TIMEOUT_SEC))
                    .build()
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };

        let storage_dir = project_root.join(".ace-tool").join(LOCAL_SEARCH_DIR);
        let chunks_dir = storage_dir.join(CHUNKS_DIR);
        let query_cache_dir = storage_dir.join(QUERY_CACHE_DIR);
        let rerank_cache_dir = storage_dir.join(RERANK_CACHE_DIR);
        log_debug(format!(
            "search_context: local init project_root={} storage_dir={} chunks_dir={} query_cache_dir={} rerank_cache_dir={} summary_mode={} rerank_mode={} rebuild_mode={}",
            project_root.display(),
            storage_dir.display(),
            chunks_dir.display(),
            query_cache_dir.display(),
            rerank_cache_dir.display(),
            summary_mode.as_str(),
            rerank_mode.as_str(),
            index_rebuild_mode.as_str()
        ));

        Ok(Self {
            project_root,
            storage_dir,
            chunks_dir,
            query_cache_dir,
            rerank_cache_dir,
            text_extensions,
            exclude_patterns,
            max_lines_per_blob,
            codex_api_base: codex_api_base.trim_end_matches('/').to_string(),
            codex_model,
            rerank_model,
            summary_mode,
            rerank_mode,
            index_rebuild_mode,
            search_timeout_sec,
            rerank_pool_size: rerank_pool_size.max(MIN_RERANK_CANDIDATES),
            rerank_timeout_sec,
            client,
        })
    }

    pub async fn search_context(&self, query: &str) -> Result<String, String> {
        let search_started = Instant::now();
        self.ensure_storage_dirs()?;
        self.best_effort_prune_query_cache("startup");
        self.best_effort_prune_rerank_cache("startup");

        let refresh_started = Instant::now();
        let index_stats = self.refresh_local_index().await?;
        log_debug(format!(
            "search_context: local index_done mode={} reason={} health={} files={} chunks={} reused={} updated={} deleted={} orphan_chunks={} query_cache_entries={} signature={} elapsed={}ms",
            index_stats.index_mode,
            index_stats.rebuild_reason,
            index_stats.storage_health,
            index_stats.file_count,
            index_stats.chunk_count,
            index_stats.reused_files,
            index_stats.updated_files,
            index_stats.deleted_files,
            index_stats.orphan_chunk_files_deleted,
            index_stats.query_cache_entries,
            shorten_hash(&index_stats.index_signature),
            refresh_started.elapsed().as_millis()
        ));

        let load_started = Instant::now();
        let chunks = self.load_all_chunks()?;
        log_debug(format!(
            "search_context: local load_done chunk_count={} elapsed={}ms",
            chunks.len(),
            load_started.elapsed().as_millis()
        ));
        if chunks.is_empty() {
            return Ok("No relevant code context found for your query.".to_string());
        }

        let prepared_query = prepare_query(query);
        if prepared_query.broad_intent || !prepared_query.hint_terms.is_empty() {
            log_debug(format!(
                "search_context: local query_expand broad_intent={} hint_terms={} hint_phrases={}",
                prepared_query.broad_intent,
                render_debug_list(&prepared_query.hint_terms),
                render_debug_list(&prepared_query.hint_phrases)
            ));
        }
        let rank_started = Instant::now();
        let ranked = rank_candidates(
            &prepared_query.terms,
            chunks,
            self.rerank_pool_size.max(DEFAULT_TOP_K) * RANK_CANDIDATE_POOL_MULTIPLIER,
        );
        let mut candidates = merge_adjacent_candidates(ranked, self.rerank_pool_size.max(DEFAULT_TOP_K));
        log_debug(format!(
            "search_context: local rank_done candidate_count={} elapsed={}ms",
            candidates.len(),
            rank_started.elapsed().as_millis()
        ));
        if candidates.is_empty() {
            return Ok("No relevant code context found for your query.".to_string());
        }

        if self.should_use_rerank(&prepared_query, &candidates) {
            let rerank_started = Instant::now();
            let rerank_candidate_signature = build_candidate_signature(&candidates);
            let rerank_timeout =
                compute_local_rerank_timeout(self.search_timeout_sec, search_started.elapsed(), self.rerank_timeout_sec);
            if let Some(rerank_timeout) = rerank_timeout {
                log_debug(format!(
                    "search_context: local rerank_start mode={} candidate_signature={} timeout_ms={} candidate_count={}",
                    self.rerank_mode.as_str(),
                    shorten_hash(&rerank_candidate_signature),
                    rerank_timeout.as_millis(),
                    candidates.len()
                ));
                match self
                    .rerank_candidates(
                        query,
                        &candidates,
                        &rerank_candidate_signature,
                        rerank_timeout,
                    )
                    .await
                {
                    Ok(reranked) => {
                        candidates = reranked;
                        log_debug(format!(
                            "search_context: local rerank_done elapsed={}ms candidate_count={}",
                            rerank_started.elapsed().as_millis(),
                            candidates.len()
                        ));
                    }
                    Err(err) => {
                        log_debug(format!(
                            "search_context: local rerank_fallback reason={}",
                            err
                        ));
                    }
                }
            } else {
                let remaining_ms =
                    remaining_budget_ms(self.search_timeout_sec, search_started.elapsed());
                log_debug(format!(
                    "search_context: local rerank_skipped reason=insufficient_budget remaining_ms={}",
                    remaining_ms
                ));
            }
        }

        candidates.truncate(DEFAULT_TOP_K);

        let candidate_signature = build_candidate_signature(&candidates);
        let cache_key =
            build_query_cache_key(query, &candidate_signature, self.summary_mode.as_str());
        if let Some(cached) = self.load_cached_query_result(
            &cache_key,
            query,
            &candidate_signature,
            self.summary_mode.as_str(),
        )? {
            log_debug(format!(
                "search_context: local cache hit query_len={} candidate_signature={}",
                query.chars().count(),
                shorten_hash(&candidate_signature)
            ));
            return Ok(cached);
        }

        let result = match self.summary_mode {
            LocalSummaryMode::Gpt => {
                let summary_started = Instant::now();
                let summary_timeout =
                    compute_local_summary_timeout(self.search_timeout_sec, search_started.elapsed());
                let Some(summary_timeout) = summary_timeout else {
                    let remaining_ms = remaining_budget_ms(
                        self.search_timeout_sec,
                        search_started.elapsed(),
                    );
                    log_debug(format!(
                        "search_context: local summary skipped reason=insufficient_budget remaining_ms={}",
                        remaining_ms
                    ));
                    return Ok(render_structured_fallback(
                        query,
                        &candidates,
                        Some(&format!(
                            "summary_mode=gpt, reason=insufficient_budget remaining_ms={}",
                            remaining_ms
                        )),
                    ));
                };
                log_debug(format!(
                    "search_context: local summary_start mode={} candidate_signature={} timeout_ms={}",
                    self.summary_mode.as_str(),
                    shorten_hash(&candidate_signature),
                    summary_timeout.as_millis()
                ));
                match self
                    .summarize_candidates(query, &candidates, summary_timeout)
                    .await
                {
                    Ok(text) if !text.trim().is_empty() => {
                        log_debug(format!(
                            "search_context: local summary_done mode={} elapsed={}ms",
                            self.summary_mode.as_str(),
                            summary_started.elapsed().as_millis()
                        ));
                        text
                    }
                    Ok(_) => {
                        log_debug(
                            "search_context: local summary fallback reason=empty_response"
                                .to_string(),
                        );
                        render_structured_fallback(
                            query,
                            &candidates,
                            Some("summary_mode=gpt, reason=empty_response"),
                        )
                    }
                    Err(err) => {
                        log_debug(format!(
                            "search_context: local summary fallback reason={}",
                            err
                        ));
                        render_structured_fallback(
                            query,
                            &candidates,
                            Some(&format!("summary_mode=gpt, reason={}", err)),
                        )
                    }
                }
            }
            LocalSummaryMode::LocalFallbackOnly => {
                log_debug(
                    "search_context: local summary skipped reason=summary_mode_local_fallback_only"
                        .to_string(),
                );
                render_structured_fallback(
                    query,
                    &candidates,
                    Some("summary_mode=local_fallback_only"),
                )
            }
        };

        self.save_cached_query_result(
            &cache_key,
            query,
            &candidate_signature,
            self.summary_mode.as_str(),
            &result,
        )?;
        self.best_effort_prune_query_cache("post_save");
        Ok(result)
    }

    fn ensure_storage_dirs(&self) -> Result<(), String> {
        fs::create_dir_all(&self.storage_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.chunks_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.query_cache_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.rerank_cache_dir).map_err(|e| e.to_string())?;
        log_debug(format!(
            "search_context: local ensure_dirs storage_dir={} chunks_dir={} query_cache_dir={} rerank_cache_dir={}",
            self.storage_dir.display(),
            self.chunks_dir.display(),
            self.query_cache_dir.display(),
            self.rerank_cache_dir.display()
        ));
        Ok(())
    }

    async fn refresh_local_index(&self) -> Result<IndexUpdateStats, String> {
        self.handle_legacy_layout()?;
        let had_manifest = self.storage_dir.join(MANIFEST_FILE).exists();
        let storage_health = self.inspect_storage_health()?;
        if storage_health.requires_full_rebuild {
            let rebuild_reason = storage_health
                .rebuild_reason
                .as_deref()
                .unwrap_or("storage_recovery");
            log_debug(format!(
                "search_context: local storage_recover_start reason={}",
                rebuild_reason
            ));
            self.reset_index_storage(true)?;
        }

        let scan_started = Instant::now();
        log_debug(format!(
            "search_context: local scan_start rebuild_mode={}",
            self.index_rebuild_mode.as_str()
        ));
        let discovered = self.scan_project_files()?;
        log_debug(format!(
            "search_context: local scan_done file_count={} elapsed={}ms",
            discovered.len(),
            scan_started.elapsed().as_millis()
        ));

        let force_full = self.index_rebuild_mode == LocalIndexRebuildMode::ForceFull
            || storage_health.requires_full_rebuild;
        let rebuild_reason = determine_rebuild_reason(
            self.index_rebuild_mode,
            had_manifest,
            storage_health.rebuild_reason.as_deref(),
        );
        self.write_index_from_discovered(
            discovered,
            force_full,
            &rebuild_reason,
            &storage_health.health_label,
        )
    }

    fn handle_legacy_layout(&self) -> Result<(), String> {
        let manifest_path = self.storage_dir.join(MANIFEST_FILE);
        if manifest_path.exists() {
            return Ok(());
        }

        let legacy_chunks_path = self.storage_dir.join(LEGACY_CHUNKS_FILE);
        if legacy_chunks_path.exists() {
            log_debug(
                "search_context: local legacy chunks.json detected; rebuilding into chunked layout"
                    .to_string(),
            );
            fs::remove_file(legacy_chunks_path).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn inspect_storage_health(&self) -> Result<StorageHealthStatus, String> {
        let manifest_path = self.storage_dir.join(MANIFEST_FILE);
        if !manifest_path.exists() {
            self.repair_meta_file_if_needed()?;
            return Ok(StorageHealthStatus::healthy("initializing"));
        }

        let manifest = match read_json_file::<LocalFileManifest>(&manifest_path) {
            Ok(value) => value,
            Err(err) => {
                log_debug(format!(
                    "search_context: local storage_recover_detected reason=manifest_unreadable error={}",
                    err
                ));
                return Ok(StorageHealthStatus::requires_rebuild(
                    "manifest_unreadable",
                ));
            }
        };
        if manifest.version != INDEX_VERSION {
            log_debug(format!(
                "search_context: local storage_recover_detected reason=manifest_version_mismatch version={}",
                manifest.version
            ));
            return Ok(StorageHealthStatus::requires_rebuild(
                "manifest_version_mismatch",
            ));
        }

        for entry in &manifest.files {
            let chunk_path = self.chunk_file_path(&entry.chunk_file);
            if !chunk_path.exists() {
                log_debug(format!(
                    "search_context: local storage_recover_detected reason=missing_chunk_file path={}",
                    entry.path
                ));
                return Ok(StorageHealthStatus::requires_rebuild("missing_chunk_file"));
            }

            match read_json_file::<Vec<LocalChunkRecord>>(&chunk_path) {
                Ok(records) => {
                    if records.is_empty() {
                        log_debug(format!(
                            "search_context: local storage_recover_detected reason=empty_chunk_file path={}",
                            entry.path
                        ));
                        return Ok(StorageHealthStatus::requires_rebuild("empty_chunk_file"));
                    }
                }
                Err(err) => {
                    log_debug(format!(
                        "search_context: local storage_recover_detected reason=chunk_unreadable path={} error={}",
                        entry.path, err
                    ));
                    return Ok(StorageHealthStatus::requires_rebuild("chunk_unreadable"));
                }
            }
        }

        self.repair_meta_file_if_needed()?;
        Ok(StorageHealthStatus::healthy("healthy"))
    }

    fn repair_meta_file_if_needed(&self) -> Result<(), String> {
        let meta_path = self.storage_dir.join(META_FILE);
        if !meta_path.exists() {
            return Ok(());
        }

        match read_json_file::<LocalIndexMeta>(&meta_path) {
            Ok(meta) if meta.version == INDEX_VERSION => Ok(()),
            Ok(meta) => {
                log_debug(format!(
                    "search_context: local meta_reset reason=version_mismatch version={}",
                    meta.version
                ));
                remove_file_if_exists(&meta_path)
            }
            Err(err) => {
                log_debug(format!(
                    "search_context: local meta_reset reason=unreadable error={}",
                    err
                ));
                remove_file_if_exists(&meta_path)
            }
        }
    }

    fn reset_index_storage(&self, clear_query_cache: bool) -> Result<(), String> {
        remove_file_if_exists(&self.storage_dir.join(MANIFEST_FILE))?;
        remove_file_if_exists(&self.storage_dir.join(META_FILE))?;
        remove_dir_if_exists(&self.chunks_dir)?;
        fs::create_dir_all(&self.chunks_dir).map_err(|e| e.to_string())?;

        if clear_query_cache {
            remove_dir_if_exists(&self.query_cache_dir)?;
            remove_dir_if_exists(&self.rerank_cache_dir)?;
        }
        fs::create_dir_all(&self.query_cache_dir).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.rerank_cache_dir).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn scan_project_files(&self) -> Result<Vec<DiscoveredFile>, String> {
        let gitignore = load_gitignore(&self.project_root);
        let mut files = Vec::new();
        let mut walker = WalkDir::new(&self.project_root).into_iter();

        while let Some(entry) = walker.next() {
            let entry = match entry {
                Ok(item) => item,
                Err(_) => continue,
            };
            let path = entry.path();
            let is_dir = entry.file_type().is_dir();

            if should_exclude_path(
                path,
                is_dir,
                &self.project_root,
                gitignore.as_ref(),
                &self.exclude_patterns,
            ) {
                if is_dir {
                    walker.skip_current_dir();
                }
                continue;
            }

            if is_dir {
                continue;
            }

            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            let ext = if ext.is_empty() {
                String::new()
            } else {
                format!(".{}", ext.to_lowercase())
            };
            if !self.text_extensions.contains(&ext) {
                continue;
            }

            let relative = match path.strip_prefix(&self.project_root) {
                Ok(rel) => rel,
                Err(_) => continue,
            };
            let file_name = relative
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let metadata = match fs::metadata(path) {
                Ok(value) => value,
                Err(_) => continue,
            };

            files.push(DiscoveredFile {
                absolute_path: path.to_path_buf(),
                path: normalize_path(relative),
                file_name,
                modified_unix_sec: metadata
                    .modified()
                    .ok()
                    .map(system_time_to_unix_seconds)
                    .unwrap_or(0),
                size: metadata.len(),
            });
        }

        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    fn write_index_from_discovered(
        &self,
        discovered: Vec<DiscoveredFile>,
        force_full: bool,
        rebuild_reason: &str,
        storage_health: &str,
    ) -> Result<IndexUpdateStats, String> {
        let old_manifest = self.load_manifest()?;
        let mut old_map = HashMap::<String, LocalFileManifestEntry>::new();
        for entry in old_manifest.files {
            old_map.insert(entry.path.clone(), entry);
        }

        let mut next_entries = Vec::new();
        let mut reused_files = 0usize;
        let mut updated_files = 0usize;
        let mut retained_paths = HashSet::new();

        for file in &discovered {
            let existing = old_map.get(&file.path);
            if !force_full && existing.is_some_and(|entry| self.can_reuse_entry(entry, file)) {
                reused_files += 1;
                retained_paths.insert(file.path.clone());
                if let Some(entry) = existing {
                    next_entries.push(entry.clone());
                }
                continue;
            }

            match self.index_file(file)? {
                Some(entry) => {
                    updated_files += 1;
                    retained_paths.insert(file.path.clone());
                    next_entries.push(entry);
                }
                None => {
                    if existing.is_some() {
                        updated_files += 1;
                    }
                }
            }
        }

        let mut deleted_files = 0usize;
        for entry in old_map.values() {
            if !retained_paths.contains(&entry.path) {
                deleted_files += 1;
                let _ = self.delete_chunk_file(&entry.chunk_file);
            }
        }

        next_entries.sort_by(|left, right| left.path.cmp(&right.path));
        let manifest = LocalFileManifest {
            version: INDEX_VERSION,
            files: next_entries,
        };
        self.save_manifest(&manifest)?;
        let referenced_chunk_files = manifest
            .files
            .iter()
            .map(|entry| entry.chunk_file.clone())
            .collect::<HashSet<_>>();
        let orphan_chunk_files_deleted =
            self.cleanup_orphan_chunk_files(&referenced_chunk_files)?;

        let index_mode = if force_full { "full" } else { "incremental" };
        let chunk_count = manifest.files.iter().map(|entry| entry.chunk_count).sum::<usize>();
        let index_signature = build_index_signature(&manifest.files);
        let query_cache_entries = self.count_query_cache_entries()?;
        let meta = LocalIndexMeta {
            version: INDEX_VERSION,
            project_root: self.project_root.to_string_lossy().replace('\\', "/"),
            retrieval_mode: "keyword".to_string(),
            summary_mode: self.summary_mode.as_str().to_string(),
            summary_model: self.codex_model.clone(),
            index_mode: index_mode.to_string(),
            chunk_lines: DEFAULT_CHUNK_LINES.min(self.max_lines_per_blob.max(1)),
            chunk_overlap_lines: DEFAULT_CHUNK_OVERLAP_LINES,
            indexed_at_unix_sec: now_unix_seconds(),
            file_count: manifest.files.len(),
            chunk_count,
            index_signature: index_signature.clone(),
            rebuild_reason: rebuild_reason.to_string(),
            storage_health: storage_health.to_string(),
            orphan_chunk_files_deleted,
            query_cache_entries,
        };
        self.save_meta(&meta)?;

        Ok(IndexUpdateStats {
            index_mode: index_mode.to_string(),
            rebuild_reason: rebuild_reason.to_string(),
            storage_health: storage_health.to_string(),
            reused_files,
            updated_files,
            deleted_files,
            orphan_chunk_files_deleted,
            file_count: manifest.files.len(),
            chunk_count,
            index_signature,
            query_cache_entries,
        })
    }

    fn can_reuse_entry(&self, entry: &LocalFileManifestEntry, file: &DiscoveredFile) -> bool {
        entry.modified_unix_sec == file.modified_unix_sec
            && entry.size == file.size
            && self.chunk_file_path(&entry.chunk_file).exists()
    }

    fn index_file(
        &self,
        file: &DiscoveredFile,
    ) -> Result<Option<LocalFileManifestEntry>, String> {
        let content = match read_file_with_encoding(&file.absolute_path) {
            Ok(data) => data,
            Err(err) => {
                log_debug(format!(
                    "search_context: local file_skip path={} reason=read_error:{}",
                    file.path, err
                ));
                return Ok(None);
            }
        };

        if is_binary_content(&content) {
            log_debug(format!(
                "search_context: local file_skip path={} reason=binary_content",
                file.path
            ));
            return Ok(None);
        }

        let clean_content = sanitize_content(&content);
        if clean_content.trim().is_empty() {
            log_debug(format!(
                "search_context: local file_skip path={} reason=empty_content",
                file.path
            ));
            return Ok(None);
        }

        let parts =
            split_content_into_local_chunks(&file.path, &clean_content, self.max_lines_per_blob);
        if parts.is_empty() {
            return Ok(None);
        }

        let chunk_file = format!("{}.json", sha256_hex(file.path.as_bytes()));
        let file_hash = sha256_hex(clean_content.as_bytes());
        let mut records = Vec::new();

        for (start_line, end_line, chunk_content) in parts {
            let normalized_text = normalize_search_text(&format!(
                "{} {} {}",
                file.path, file.file_name, chunk_content
            ));
            let token_freq = build_token_freq(&normalized_text);
            let token_count = token_freq.values().map(|v| *v as usize).sum::<usize>();
            let content_hash = sha256_hex(chunk_content.as_bytes());
            let id = sha256_hex(
                format!("{}:{}:{}:{}", file.path, start_line, end_line, content_hash).as_bytes(),
            );
            let symbol_tokens = extract_symbol_tokens(&file.path, &chunk_content);

            records.push(LocalChunkRecord {
                id,
                path: file.path.clone(),
                file_name: file.file_name.clone(),
                start_line,
                end_line,
                content_hash,
                content: chunk_content,
                normalized_text,
                token_freq,
                token_count,
                symbol_tokens,
            });
        }

        self.write_chunk_file(&chunk_file, &records)?;

        Ok(Some(LocalFileManifestEntry {
            path: file.path.clone(),
            file_name: file.file_name.clone(),
            modified_unix_sec: file.modified_unix_sec,
            size: file.size,
            file_hash,
            chunk_file,
            chunk_count: records.len(),
        }))
    }

    async fn summarize_candidates(
        &self,
        query: &str,
        candidates: &[QueryCandidate],
        request_timeout: Duration,
    ) -> Result<String, String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "summary client is unavailable in current local summary mode".to_string())?;
        let context = render_context_for_summary(candidates);
        let payload = build_local_summary_payload(&self.codex_model, query, &context);
        let payload_text = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        log_debug(format!(
            "search_context: local summary_request model={} query_len={} context_chars={} payload_bytes={} timeout_ms={}",
            self.codex_model,
            query.chars().count(),
            context.chars().count(),
            payload_text.len(),
            request_timeout.as_millis()
        ));
        log_debug_verbose(format!(
            "search_context: local summary_payload_preview={}",
            sanitize_response_text(&payload_text)
        ));
        let request_started = Instant::now();

        let response = client
            .post(format!("{}/chat/completions", self.codex_api_base))
            .timeout(request_timeout)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                let kind = if e.is_timeout() {
                    "timeout"
                } else if e.is_connect() {
                    "connect"
                } else if e.is_request() {
                    "request"
                } else if e.is_body() {
                    "body"
                } else if e.is_decode() {
                    "decode"
                } else {
                    "other"
                };
                format!(
                    "Local search summary request failed kind={} elapsed_ms={} error={}",
                    kind,
                    request_started.elapsed().as_millis(),
                    e
                )
            })?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "Local search summary request failed with status {}. Response: {}",
                status.as_u16(),
                sanitize_response_text(&response_text)
            ));
        }

        parse_chat_completion_text(&response_text)
    }

    fn should_use_rerank(&self, prepared_query: &PreparedQuery, candidates: &[QueryCandidate]) -> bool {
        self.rerank_mode == LocalRerankMode::BroadOnly
            && prepared_query.broad_intent
            && candidates.len() >= MIN_RERANK_CANDIDATES
    }

    async fn rerank_candidates(
        &self,
        query: &str,
        candidates: &[QueryCandidate],
        candidate_signature: &str,
        request_timeout: Duration,
    ) -> Result<Vec<QueryCandidate>, String> {
        let client = self
            .client
            .as_ref()
            .ok_or_else(|| "rerank client is unavailable in current local rerank mode".to_string())?;

        let cache_key =
            build_rerank_cache_key(query, candidate_signature, self.rerank_mode.as_str(), &self.rerank_model);
        if let Some(cached_ids) = self.load_cached_rerank_result(
            &cache_key,
            query,
            candidate_signature,
            self.rerank_mode.as_str(),
            &self.rerank_model,
        )? {
            log_debug(format!(
                "search_context: local rerank_cache_hit query_len={} candidate_signature={}",
                query.chars().count(),
                shorten_hash(candidate_signature)
            ));
            return Ok(apply_rerank_order(candidates, &cached_ids));
        }

        let rerank_candidates = candidates
            .iter()
            .take(self.rerank_pool_size)
            .map(build_rerank_candidate)
            .collect::<Vec<_>>();
        let context = render_context_for_rerank(&rerank_candidates);
        let payload = build_local_rerank_payload(&self.rerank_model, query, &context);
        let payload_text = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
        log_debug(format!(
            "search_context: local rerank_request model={} query_len={} context_chars={} payload_bytes={} timeout_ms={} candidate_count={}",
            self.rerank_model,
            query.chars().count(),
            context.chars().count(),
            payload_text.len(),
            request_timeout.as_millis(),
            rerank_candidates.len()
        ));
        let request_started = Instant::now();
        let response = client
            .post(format!("{}/chat/completions", self.codex_api_base))
            .timeout(request_timeout)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                let kind = if e.is_timeout() {
                    "timeout"
                } else if e.is_connect() {
                    "connect"
                } else if e.is_request() {
                    "request"
                } else if e.is_body() {
                    "body"
                } else if e.is_decode() {
                    "decode"
                } else {
                    "other"
                };
                format!(
                    "Local rerank request failed kind={} elapsed_ms={} error={}",
                    kind,
                    request_started.elapsed().as_millis(),
                    e
                )
            })?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!(
                "Local rerank request failed with status {}. Response: {}",
                status.as_u16(),
                sanitize_response_text(&response_text)
            ));
        }

        let ordered_ids = parse_rerank_result(&response_text, &rerank_candidates)?;
        self.save_cached_rerank_result(
            &cache_key,
            query,
            candidate_signature,
            self.rerank_mode.as_str(),
            &self.rerank_model,
            &ordered_ids,
        )?;
        self.best_effort_prune_rerank_cache("post_save");
        Ok(apply_rerank_order(candidates, &ordered_ids))
    }

    fn load_manifest(&self) -> Result<LocalFileManifest, String> {
        let path = self.storage_dir.join(MANIFEST_FILE);
        if !path.exists() {
            return Ok(LocalFileManifest {
                version: INDEX_VERSION,
                files: Vec::new(),
            });
        }
        read_json_file(&path)
    }

    fn save_manifest(&self, manifest: &LocalFileManifest) -> Result<(), String> {
        write_json_file(&self.storage_dir.join(MANIFEST_FILE), manifest)
    }

    fn save_meta(&self, meta: &LocalIndexMeta) -> Result<(), String> {
        write_json_file(&self.storage_dir.join(META_FILE), meta)
    }

    fn load_all_chunks(&self) -> Result<Vec<LocalChunkRecord>, String> {
        let manifest = self.load_manifest()?;
        let mut chunks = Vec::new();
        for entry in manifest.files {
            match self.read_chunk_file(&entry.chunk_file) {
                Ok(mut records) => chunks.append(&mut records),
                Err(err) => {
                    log_debug(format!(
                        "search_context: local chunk_load_skip path={} reason={}",
                        entry.path, err
                    ));
                }
            }
        }
        Ok(chunks)
    }

    fn read_chunk_file(&self, chunk_file: &str) -> Result<Vec<LocalChunkRecord>, String> {
        read_json_file(&self.chunk_file_path(chunk_file))
    }

    fn write_chunk_file(
        &self,
        chunk_file: &str,
        records: &[LocalChunkRecord],
    ) -> Result<(), String> {
        write_json_file(&self.chunk_file_path(chunk_file), records)
    }

    fn delete_chunk_file(&self, chunk_file: &str) -> Result<(), String> {
        let path = self.chunk_file_path(chunk_file);
        if !path.exists() {
            return Ok(());
        }
        fs::remove_file(path).map_err(|e| e.to_string())
    }

    fn chunk_file_path(&self, chunk_file: &str) -> PathBuf {
        self.chunks_dir.join(chunk_file)
    }

    fn cleanup_orphan_chunk_files(
        &self,
        referenced_chunk_files: &HashSet<String>,
    ) -> Result<usize, String> {
        if !self.chunks_dir.exists() {
            return Ok(0);
        }

        let mut deleted = 0usize;
        let entries = fs::read_dir(&self.chunks_dir).map_err(|e| e.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let file_name = match path.file_name().and_then(|value| value.to_str()) {
                Some(value) => value.to_string(),
                None => continue,
            };
            if referenced_chunk_files.contains(&file_name) {
                continue;
            }
            fs::remove_file(&path).map_err(|e| e.to_string())?;
            deleted += 1;
        }

        Ok(deleted)
    }

    fn count_query_cache_entries(&self) -> Result<usize, String> {
        if !self.query_cache_dir.exists() {
            return Ok(0);
        }

        let mut count = 0usize;
        let entries = fs::read_dir(&self.query_cache_dir).map_err(|e| e.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("json"))
            {
                count += 1;
            }
        }
        Ok(count)
    }

    fn load_cached_query_result(
        &self,
        cache_key: &str,
        query: &str,
        candidate_signature: &str,
        summary_mode: &str,
    ) -> Result<Option<String>, String> {
        let path = self.query_cache_path(cache_key);
        if !path.exists() {
            return Ok(None);
        }
        let entry = match read_json_file::<QueryCacheEntry>(&path) {
            Ok(value) => value,
            Err(err) => {
                log_debug(format!(
                    "search_context: local query_cache_drop reason=invalid key={} error={}",
                    shorten_hash(cache_key),
                    err
                ));
                let _ = remove_file_if_exists(&path);
                return Ok(None);
            }
        };
        if is_query_cache_entry_expired(&entry, now_unix_seconds()) {
            log_debug(format!(
                "search_context: local query_cache_drop reason=expired key={}",
                shorten_hash(cache_key)
            ));
            let _ = remove_file_if_exists(&path);
            return Ok(None);
        }
        if entry.query == query.trim()
            && entry.candidate_signature == candidate_signature
            && entry.summary_mode == summary_mode
        {
            Ok(Some(entry.result))
        } else {
            Ok(None)
        }
    }

    fn save_cached_query_result(
        &self,
        cache_key: &str,
        query: &str,
        candidate_signature: &str,
        summary_mode: &str,
        result: &str,
    ) -> Result<(), String> {
        let entry = QueryCacheEntry {
            query: query.trim().to_string(),
            candidate_signature: candidate_signature.to_string(),
            summary_mode: summary_mode.to_string(),
            result: result.to_string(),
            cached_at_unix_sec: now_unix_seconds(),
        };
        write_json_file(&self.query_cache_path(cache_key), &entry)
    }

    fn query_cache_path(&self, cache_key: &str) -> PathBuf {
        self.query_cache_dir.join(format!("{}.json", cache_key))
    }

    fn rerank_cache_path(&self, cache_key: &str) -> PathBuf {
        self.rerank_cache_dir.join(format!("{}.json", cache_key))
    }

    fn prune_query_cache(&self) -> Result<QueryCachePruneStats, String> {
        if !self.query_cache_dir.exists() {
            return Ok(QueryCachePruneStats::default());
        }

        let now = now_unix_seconds();
        let mut stats = QueryCachePruneStats::default();
        let mut valid_entries = Vec::<(PathBuf, u64)>::new();
        let entries = fs::read_dir(&self.query_cache_dir).map_err(|e| e.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| !value.eq_ignore_ascii_case("json"))
            {
                continue;
            }

            match read_json_file::<QueryCacheEntry>(&path) {
                Ok(value) => {
                    if is_query_cache_entry_expired(&value, now) {
                        remove_file_if_exists(&path)?;
                        stats.expired_removed += 1;
                    } else {
                        valid_entries.push((path, value.cached_at_unix_sec));
                    }
                }
                Err(_) => {
                    remove_file_if_exists(&path)?;
                    stats.invalid_removed += 1;
                }
            }
        }

        valid_entries.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        if valid_entries.len() > QUERY_CACHE_MAX_ENTRIES {
            for (path, _) in valid_entries.iter().skip(QUERY_CACHE_MAX_ENTRIES) {
                remove_file_if_exists(path)?;
                stats.overflow_removed += 1;
            }
        }
        stats.kept_entries = valid_entries.len().min(QUERY_CACHE_MAX_ENTRIES);
        Ok(stats)
    }

    fn prune_rerank_cache(&self) -> Result<QueryCachePruneStats, String> {
        if !self.rerank_cache_dir.exists() {
            return Ok(QueryCachePruneStats::default());
        }

        let now = now_unix_seconds();
        let mut stats = QueryCachePruneStats::default();
        let mut valid_entries = Vec::<(PathBuf, u64)>::new();
        let entries = fs::read_dir(&self.rerank_cache_dir).map_err(|e| e.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| !value.eq_ignore_ascii_case("json"))
            {
                continue;
            }

            match read_json_file::<RerankCacheEntry>(&path) {
                Ok(value) => {
                    if now.saturating_sub(value.cached_at_unix_sec) > RERANK_CACHE_TTL_SEC {
                        remove_file_if_exists(&path)?;
                        stats.expired_removed += 1;
                    } else {
                        valid_entries.push((path, value.cached_at_unix_sec));
                    }
                }
                Err(_) => {
                    remove_file_if_exists(&path)?;
                    stats.invalid_removed += 1;
                }
            }
        }

        valid_entries.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        if valid_entries.len() > RERANK_CACHE_MAX_ENTRIES {
            for (path, _) in valid_entries.iter().skip(RERANK_CACHE_MAX_ENTRIES) {
                remove_file_if_exists(path)?;
                stats.overflow_removed += 1;
            }
        }
        stats.kept_entries = valid_entries.len().min(RERANK_CACHE_MAX_ENTRIES);
        Ok(stats)
    }

    fn best_effort_prune_query_cache(&self, stage: &str) {
        match self.prune_query_cache() {
            Ok(stats) if stats.total_removed() > 0 => {
                log_debug(format!(
                    "search_context: local query_cache_pruned stage={} kept={} expired={} invalid={} overflow={}",
                    stage,
                    stats.kept_entries,
                    stats.expired_removed,
                    stats.invalid_removed,
                    stats.overflow_removed
                ));
            }
            Ok(_) => {}
            Err(err) => {
                log_debug(format!(
                    "search_context: local query_cache_prune_error stage={} error={}",
                    stage, err
                ));
            }
        }
    }

    fn best_effort_prune_rerank_cache(&self, stage: &str) {
        match self.prune_rerank_cache() {
            Ok(stats) if stats.total_removed() > 0 => {
                log_debug(format!(
                    "search_context: local rerank_cache_pruned stage={} kept={} expired={} invalid={} overflow={}",
                    stage,
                    stats.kept_entries,
                    stats.expired_removed,
                    stats.invalid_removed,
                    stats.overflow_removed
                ));
            }
            Ok(_) => {}
            Err(err) => {
                log_debug(format!(
                    "search_context: local rerank_cache_prune_error stage={} error={}",
                    stage, err
                ));
            }
        }
    }

    fn load_cached_rerank_result(
        &self,
        cache_key: &str,
        query: &str,
        candidate_signature: &str,
        rerank_mode: &str,
        rerank_model: &str,
    ) -> Result<Option<Vec<String>>, String> {
        let path = self.rerank_cache_path(cache_key);
        if !path.exists() {
            return Ok(None);
        }
        let entry = match read_json_file::<RerankCacheEntry>(&path) {
            Ok(value) => value,
            Err(err) => {
                log_debug(format!(
                    "search_context: local rerank_cache_drop reason=invalid key={} error={}",
                    shorten_hash(cache_key),
                    err
                ));
                let _ = remove_file_if_exists(&path);
                return Ok(None);
            }
        };
        if now_unix_seconds().saturating_sub(entry.cached_at_unix_sec) > RERANK_CACHE_TTL_SEC {
            log_debug(format!(
                "search_context: local rerank_cache_drop reason=expired key={}",
                shorten_hash(cache_key)
            ));
            let _ = remove_file_if_exists(&path);
            return Ok(None);
        }
        if entry.query == query.trim()
            && entry.candidate_signature == candidate_signature
            && entry.rerank_mode == rerank_mode
            && entry.rerank_model == rerank_model
        {
            Ok(Some(entry.ordered_candidate_ids))
        } else {
            Ok(None)
        }
    }

    fn save_cached_rerank_result(
        &self,
        cache_key: &str,
        query: &str,
        candidate_signature: &str,
        rerank_mode: &str,
        rerank_model: &str,
        ordered_candidate_ids: &[String],
    ) -> Result<(), String> {
        let entry = RerankCacheEntry {
            query: query.trim().to_string(),
            candidate_signature: candidate_signature.to_string(),
            rerank_model: rerank_model.to_string(),
            rerank_mode: rerank_mode.to_string(),
            ordered_candidate_ids: ordered_candidate_ids.to_vec(),
            cached_at_unix_sec: now_unix_seconds(),
        };
        write_json_file(&self.rerank_cache_path(cache_key), &entry)
    }
}

fn read_json_file<T>(path: &Path) -> Result<T, String>
where
    T: DeserializeOwned,
{
    let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str::<T>(&content).map_err(|e| e.to_string())
}

fn write_json_file<T>(path: &Path, value: &T) -> Result<(), String>
where
    T: Serialize + ?Sized,
{
    let content = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}

fn split_content_into_local_chunks(
    path: &str,
    content: &str,
    max_lines_per_blob: usize,
) -> Vec<(usize, usize, String)> {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let chunk_lines = DEFAULT_CHUNK_LINES.min(max_lines_per_blob.max(1));
    let overlap = DEFAULT_CHUNK_OVERLAP_LINES.min(chunk_lines / 3).max(10);
    let step = chunk_lines.saturating_sub(overlap).max(1);
    let mut start = 0usize;
    let mut chunks = Vec::new();

    while start < lines.len() {
        let end = (start + chunk_lines).min(lines.len());
        let joined = lines[start..end].join("\n").trim().to_string();
        if !joined.is_empty() {
            chunks.push((start + 1, end, joined));
        }
        if end == lines.len() {
            break;
        }
        start += step;
    }

    if chunks.is_empty() {
        log_debug(format!("search_context: local skip empty chunk path={}", path));
    }
    chunks
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(path).map_err(|e| e.to_string())
}

fn remove_dir_if_exists(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    fs::remove_dir_all(path).map_err(|e| e.to_string())
}

fn rank_candidates(
    query_terms: &QueryTerms,
    chunks: Vec<LocalChunkRecord>,
    top_k: usize,
) -> Vec<QueryCandidate> {
    if query_terms.weighted_terms.is_empty() {
        return Vec::new();
    }

    let total_docs = chunks.len().max(1) as f32;
    let mut doc_freq = HashMap::<String, usize>::new();
    for chunk in &chunks {
        let unique_terms: HashSet<&str> = chunk.token_freq.keys().map(|key| key.as_str()).collect();
        for term in unique_terms {
            *doc_freq.entry(term.to_string()).or_insert(0) += 1;
        }
    }

    let average_len = chunks
        .iter()
        .map(|chunk| chunk.token_count as f32)
        .sum::<f32>()
        / total_docs.max(1.0);

    let mut candidates: Vec<QueryCandidate> = chunks
        .into_iter()
        .filter_map(|chunk| {
            let matched =
                compute_query_match(query_terms, &chunk, &doc_freq, total_docs, average_len);
            if matched.score <= 0.0 {
                None
            } else {
                Some(QueryCandidate {
                    score: matched.score,
                    chunk,
                    matched_terms: matched.matched_terms,
                    matched_phrases: matched.matched_phrases,
                    reason_labels: matched.reason_labels,
                })
            }
        })
        .collect();

    candidates.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
    });
    candidates.truncate(top_k);
    candidates
}

fn prepare_query(query: &str) -> PreparedQuery {
    let ordered_terms = tokenize_search_text(query)
        .into_iter()
        .filter(|term| term.len() >= 2)
        .collect::<Vec<_>>();
    let broad_intent = looks_like_broad_query(query, &ordered_terms);
    let (hint_terms, hint_phrases) = collect_query_hints(query, broad_intent);
    let terms = if !broad_intent && hint_terms.is_empty() && hint_phrases.is_empty() {
        build_query_terms(query)
    } else {
        build_query_terms_from_parts(&ordered_terms, &hint_terms, &hint_phrases)
    };
    PreparedQuery {
        terms,
        broad_intent,
        hint_terms,
        hint_phrases,
    }
}

fn compute_query_match(
    query_terms: &QueryTerms,
    chunk: &LocalChunkRecord,
    doc_freq: &HashMap<String, usize>,
    total_docs: f32,
    average_len: f32,
) -> QueryMatchOutcome {
    let mut score = 0.0f32;
    let mut matched_terms = HashSet::new();
    let mut matched_phrases = HashSet::new();
    let mut reason_labels = HashSet::new();
    let k1 = 1.2f32;
    let b = 0.75f32;
    let doc_len = chunk.token_count.max(1) as f32;
    let normalized_path = normalize_search_text(&chunk.path);
    let normalized_file = normalize_search_text(&chunk.file_name);
    let symbol_set: HashSet<&str> = chunk.symbol_tokens.iter().map(|value| value.as_str()).collect();

    for weighted in &query_terms.weighted_terms {
        let term = weighted.term.as_str();
        let tf = *chunk.token_freq.get(term).unwrap_or(&0) as f32;
        let df = *doc_freq.get(term).unwrap_or(&0) as f32;
        let idf = (((total_docs - df + 0.5) / (df + 0.5)) + 1.0).ln().max(0.0);

        if tf > 0.0 {
            let denom = tf + k1 * (1.0 - b + b * (doc_len / average_len.max(1.0)));
            score += weighted.weight * idf * (tf * (k1 + 1.0) / denom.max(f32::EPSILON));
            matched_terms.insert(term.to_string());
            reason_labels.insert("正文命中".to_string());
        }

        if symbol_set.contains(term) {
            score += weighted.weight * 2.3;
            matched_terms.insert(term.to_string());
            reason_labels.insert("符号命中".to_string());
        }

        if normalized_file.contains(term) {
            score += weighted.weight * 2.0;
            matched_terms.insert(term.to_string());
            reason_labels.insert("文件名命中".to_string());
        }

        if normalized_path.contains(term) {
            score += weighted.weight * 1.4;
            matched_terms.insert(term.to_string());
            reason_labels.insert("路径命中".to_string());
        }
    }

    for phrase in &query_terms.phrases {
        if chunk.normalized_text.contains(&phrase.phrase) {
            score += phrase.weight;
            matched_phrases.insert(phrase.phrase.clone());
            reason_labels.insert("短语命中".to_string());
        }
    }

    QueryMatchOutcome {
        score,
        matched_terms: sort_string_set(matched_terms),
        matched_phrases: sort_string_set(matched_phrases),
        reason_labels: sort_string_set(reason_labels),
    }
}

fn merge_adjacent_candidates(
    candidates: Vec<QueryCandidate>,
    top_k: usize,
) -> Vec<QueryCandidate> {
    if candidates.is_empty() {
        return candidates;
    }

    let mut ordered = candidates;
    ordered.sort_by(|left, right| match left.chunk.path.cmp(&right.chunk.path) {
        Ordering::Equal => left.chunk.start_line.cmp(&right.chunk.start_line),
        other => other,
    });

    let mut merged = Vec::<QueryCandidate>::new();
    for candidate in ordered {
        if let Some(last) = merged.last_mut() {
            let same_file = last.chunk.path == candidate.chunk.path;
            let near_by =
                candidate.chunk.start_line <= last.chunk.end_line + DEFAULT_CHUNK_OVERLAP_LINES + 5;
            if same_file && near_by {
                last.score += candidate.score;
                last.chunk.end_line = last.chunk.end_line.max(candidate.chunk.end_line);
                last.chunk.content_hash = sha256_hex(
                    format!("{}:{}", last.chunk.content_hash, candidate.chunk.content_hash)
                        .as_bytes(),
                );
                last.chunk.id = sha256_hex(
                    format!("{}:{}:{}", last.chunk.id, candidate.chunk.id, last.chunk.end_line)
                        .as_bytes(),
                );
                last.chunk.content = merge_chunk_content(&last.chunk.content, &candidate.chunk.content);
                last.chunk.token_count += candidate.chunk.token_count;
                merge_string_vec(&mut last.chunk.symbol_tokens, &candidate.chunk.symbol_tokens);
                merge_string_vec(&mut last.matched_terms, &candidate.matched_terms);
                merge_string_vec(&mut last.matched_phrases, &candidate.matched_phrases);
                merge_string_vec(&mut last.reason_labels, &candidate.reason_labels);
                continue;
            }
        }
        merged.push(candidate);
    }

    merged.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
    });
    select_diverse_candidates(merged, top_k, SOFT_MAX_RESULTS_PER_FILE)
}

fn select_diverse_candidates(
    candidates: Vec<QueryCandidate>,
    top_k: usize,
    soft_limit_per_file: usize,
) -> Vec<QueryCandidate> {
    if candidates.len() <= top_k || soft_limit_per_file == 0 {
        return candidates.into_iter().take(top_k).collect();
    }

    let mut per_file_counts = HashMap::<String, usize>::new();
    let mut selected = Vec::new();
    let mut deferred = Vec::new();

    for candidate in candidates {
        let counter = per_file_counts
            .entry(candidate.chunk.path.clone())
            .or_insert(0);
        if *counter < soft_limit_per_file {
            *counter += 1;
            selected.push(candidate);
        } else {
            deferred.push(candidate);
        }
        if selected.len() == top_k {
            return selected;
        }
    }

    for candidate in deferred {
        selected.push(candidate);
        if selected.len() == top_k {
            break;
        }
    }

    selected
}

fn build_rerank_candidate(candidate: &QueryCandidate) -> RerankCandidate {
    RerankCandidate {
        id: candidate.chunk.id.clone(),
        path: candidate.chunk.path.clone(),
        start_line: candidate.chunk.start_line,
        end_line: candidate.chunk.end_line,
        reason_summary: render_candidate_reason_summary(candidate),
        snippet: build_summary_snippet(&candidate.chunk.content),
    }
}

fn build_query_terms(query: &str) -> QueryTerms {
    let ordered_terms = tokenize_search_text(query)
        .into_iter()
        .filter(|term| term.len() >= 2)
        .collect::<Vec<_>>();
    build_query_terms_from_parts(&ordered_terms, &[], &[])
}

fn build_query_terms_from_parts(
    ordered_terms: &[String],
    hint_terms: &[String],
    hint_phrases: &[String],
) -> QueryTerms {
    let mut weighted_terms = Vec::new();
    let mut term_seen = HashSet::new();

    for term in ordered_terms {
        if term_seen.insert(term.clone()) {
            weighted_terms.push(WeightedTerm {
                term: term.clone(),
                weight: weight_for_term(term),
            });
        }
    }

    for term in hint_terms {
        if term.len() < 2 {
            continue;
        }
        if term_seen.insert(term.clone()) {
            weighted_terms.push(WeightedTerm {
                term: term.clone(),
                weight: hint_weight_for_term(term),
            });
        }
    }

    let mut phrases = Vec::new();
    let mut phrase_seen = HashSet::new();
    for size in [2usize, 3usize] {
        if ordered_terms.len() < size {
            continue;
        }
        for window in ordered_terms.windows(size) {
            let phrase = window.join(" ");
            if phrase_seen.insert(phrase.clone()) {
                let base = window.iter().map(|term| weight_for_term(term)).sum::<f32>();
                let weight = if size == 2 { base * 1.2 } else { base * 1.5 };
                phrases.push(WeightedPhrase { phrase, weight });
            }
        }
    }

    for phrase in hint_phrases {
        let normalized_tokens = tokenize_search_text(phrase)
            .into_iter()
            .filter(|term| term.len() >= 2)
            .collect::<Vec<_>>();
        if normalized_tokens.len() < 2 {
            continue;
        }
        let normalized_phrase = normalized_tokens.join(" ");
        if phrase_seen.insert(normalized_phrase.clone()) {
            let base = normalized_tokens
                .iter()
                .map(|term| hint_weight_for_term(term))
                .sum::<f32>();
            phrases.push(WeightedPhrase {
                phrase: normalized_phrase,
                weight: base * 0.9,
            });
        }
    }

    QueryTerms {
        weighted_terms,
        phrases,
    }
}

fn weight_for_term(term: &str) -> f32 {
    if contains_cjk(term) {
        1.8
    } else if term.len() >= 12 {
        1.6
    } else if term.len() >= 8 {
        1.4
    } else {
        1.0
    }
}

fn hint_weight_for_term(term: &str) -> f32 {
    (weight_for_term(term) * 0.72).max(0.65)
}

fn looks_like_broad_query(query: &str, ordered_terms: &[String]) -> bool {
    let lowered = query.trim().to_ascii_lowercase();
    let has_code_anchor = lowered.contains(".rs")
        || lowered.contains(".toml")
        || lowered.contains(".json")
        || lowered.contains("::")
        || lowered.contains('/')
        || lowered.contains('\\')
        || lowered.contains('_');
    if has_code_anchor {
        return false;
    }

    let explicit_ascii_terms = ordered_terms
        .iter()
        .filter(|term| term.chars().any(|ch| ch.is_ascii_alphabetic()) && term.len() >= 5)
        .count();
    if explicit_ascii_terms >= 2 {
        return false;
    }

    let broad_markers = [
        "配置",
        "优化",
        "改造",
        "机制",
        "流程",
        "超时",
        "本地检索",
        "提示词",
        "写到",
        "缓存",
        "索引",
    ];
    broad_markers.iter().any(|marker| query.contains(marker))
}

fn collect_query_hints(query: &str, broad_intent: bool) -> (Vec<String>, Vec<String>) {
    let lowered = query.trim().to_ascii_lowercase();
    let mut terms = Vec::new();
    let mut phrases = Vec::new();

    if query.contains("本地检索")
        || query.contains("本地搜索")
        || lowered.contains("local search")
        || lowered.contains("local_search")
    {
        push_unique_values(
            &mut terms,
            &[
                "local",
                "search",
                "context",
                "index",
                "chunk",
                "manifest",
                "query",
                "cache",
                "storage",
            ],
        );
        push_unique_values(
            &mut phrases,
            &["local search", "query cache", "project root"],
        );
    }

    if query.contains("提示词")
        || query.contains("增强")
        || lowered.contains("enhance")
        || lowered.contains("prompt")
    {
        push_unique_values(
            &mut terms,
            &[
                "enhance",
                "prompt",
                "provider",
                "codex",
                "chat",
                "completions",
            ],
        );
        push_unique_values(
            &mut phrases,
            &["enhance prompt", "chat completions", "codex provider"],
        );
    }

    if lowered.contains("codex") || lowered.contains("gpt") {
        push_unique_values(
            &mut terms,
            &[
                "codex",
                "provider",
                "config",
                "model",
                "api",
                "key",
                "base",
                "timeout",
            ],
        );
        push_unique_values(
            &mut phrases,
            &["codex provider", "chat completions"],
        );
    }

    if query.contains("配置") || lowered.contains("config") {
        push_unique_values(
            &mut terms,
            &[
                "config",
                "provider",
                "model",
                "timeout",
                "api",
                "key",
                "base",
                "env",
            ],
        );
        push_unique_values(
            &mut phrases,
            &["project root", "search timeout", "chat completions"],
        );
    }

    if query.contains("超时") || lowered.contains("timeout") {
        push_unique_values(
            &mut terms,
            &["timeout", "summary", "search", "context", "request", "retry"],
        );
        push_unique_values(
            &mut phrases,
            &["search timeout", "summary timeout"],
        );
    }

    if query.contains(".ace-tool")
        || query.contains("写到")
        || query.contains("项目根")
        || lowered.contains("project root")
    {
        push_unique_values(
            &mut terms,
            &[
                "project",
                "root",
                "storage",
                "manifest",
                "meta",
                "chunks",
                "query",
                "cache",
            ],
        );
        push_unique_values(
            &mut phrases,
            &["project root", "query cache", "local search"],
        );
    }

    if query.contains("索引")
        || query.contains("缓存")
        || query.contains("chunk")
        || lowered.contains("index")
        || lowered.contains("cache")
        || lowered.contains("manifest")
    {
        push_unique_values(
            &mut terms,
            &["index", "chunk", "manifest", "meta", "query", "cache"],
        );
        push_unique_values(
            &mut phrases,
            &["query cache", "local search"],
        );
    }

    if broad_intent && terms.is_empty() {
        push_unique_values(
            &mut terms,
            &["search", "context", "provider", "config", "local"],
        );
    }

    (terms, phrases)
}

fn compute_local_rerank_timeout(
    search_timeout_sec: u64,
    elapsed: Duration,
    rerank_timeout_sec: u64,
) -> Option<Duration> {
    let remaining = Duration::from_secs(search_timeout_sec).saturating_sub(elapsed);
    let usable_budget =
        remaining.saturating_sub(Duration::from_secs(RERANK_RESPONSE_GUARD_SEC));
    if usable_budget < Duration::from_secs(MIN_RERANK_BUDGET_SEC) {
        return None;
    }
    Some(usable_budget.min(Duration::from_secs(
        rerank_timeout_sec.max(DEFAULT_RERANK_TIMEOUT_SEC),
    )))
}

fn push_unique_values(target: &mut Vec<String>, values: &[&str]) {
    let mut seen = target.iter().cloned().collect::<HashSet<_>>();
    for value in values {
        if seen.insert((*value).to_string()) {
            target.push((*value).to_string());
        }
    }
}

fn compute_local_summary_timeout(
    search_timeout_sec: u64,
    elapsed: Duration,
) -> Option<Duration> {
    let remaining = Duration::from_secs(search_timeout_sec).saturating_sub(elapsed);
    let usable_budget =
        remaining.saturating_sub(Duration::from_secs(SUMMARY_RESPONSE_GUARD_SEC));
    if usable_budget < Duration::from_secs(MIN_SUMMARY_BUDGET_SEC) {
        return None;
    }
    Some(
        usable_budget
            .min(Duration::from_secs(DEFAULT_SUMMARY_SOFT_TIMEOUT_SEC))
            .min(Duration::from_secs(DEFAULT_SUMMARY_TIMEOUT_SEC)),
    )
}

fn remaining_budget_ms(search_timeout_sec: u64, elapsed: Duration) -> u128 {
    Duration::from_secs(search_timeout_sec)
        .saturating_sub(elapsed)
        .as_millis()
}

fn render_debug_list(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(",")
    }
}

fn normalize_search_text(text: &str) -> String {
    expand_identifier_boundaries(text)
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ('\u{4e00}'..='\u{9fff}').contains(&ch) {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
}

fn expand_identifier_boundaries(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut expanded = String::new();

    for (idx, ch) in chars.iter().enumerate() {
        if idx > 0 {
            let prev = chars[idx - 1];
            let next = chars.get(idx + 1).copied();
            if should_insert_identifier_boundary(prev, *ch, next) {
                expanded.push(' ');
            }
        }
        expanded.push(*ch);
    }

    expanded
}

fn should_insert_identifier_boundary(prev: char, current: char, next: Option<char>) -> bool {
    (prev.is_ascii_lowercase() && current.is_ascii_uppercase())
        || (prev.is_ascii_alphabetic() && current.is_ascii_digit())
        || (prev.is_ascii_digit() && current.is_ascii_alphabetic())
        || (prev.is_ascii_uppercase()
            && current.is_ascii_uppercase()
            && next.is_some_and(|value| value.is_ascii_lowercase()))
}

fn tokenize_search_text(text: &str) -> Vec<String> {
    let normalized = normalize_search_text(text);
    let mut tokens = Vec::new();
    for part in normalized.split_whitespace() {
        if part.is_empty() {
            continue;
        }
        if contains_cjk(part) {
            tokens.extend(split_cjk_term(part));
        } else {
            tokens.push(part.to_string());
        }
    }
    tokens
}

fn split_cjk_term(term: &str) -> Vec<String> {
    let chars: Vec<char> = term.chars().collect();
    if chars.len() <= 2 {
        return vec![term.to_string()];
    }

    let mut parts = Vec::new();
    for window in 0..chars.len() {
        let end = (window + 2).min(chars.len());
        if end > window {
            parts.push(chars[window..end].iter().collect::<String>());
        }
        let end3 = (window + 3).min(chars.len());
        if end3 > window + 1 {
            parts.push(chars[window..end3].iter().collect::<String>());
        }
    }
    parts.sort();
    parts.dedup();
    parts
}

fn build_token_freq(normalized_text: &str) -> HashMap<String, u32> {
    let mut freq = HashMap::new();
    for token in tokenize_search_text(normalized_text) {
        if token.trim().is_empty() {
            continue;
        }
        *freq.entry(token).or_insert(0) += 1;
    }
    freq
}

fn extract_symbol_tokens(path: &str, content: &str) -> Vec<String> {
    let mut symbols = HashSet::new();
    let file_name = Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path);
    let file_stem = Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    for token in tokenize_search_text(file_stem) {
        if token.len() >= 2 {
            symbols.insert(token);
        }
    }

    let prefixes = [
        "pub fn ",
        "fn ",
        "pub struct ",
        "struct ",
        "pub enum ",
        "enum ",
        "pub trait ",
        "trait ",
        "impl ",
        "class ",
        "interface ",
        "type ",
        "export function ",
        "function ",
        "const ",
        "let ",
    ];

    for line in content.lines() {
        let trimmed = line.trim_start();
        for prefix in prefixes {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let identifier = take_identifier(rest);
                if identifier.len() >= 2 {
                    for token in tokenize_search_text(identifier) {
                        if token.len() >= 2 {
                            symbols.insert(token);
                        }
                    }
                }
            }
        }
    }

    sort_string_set(symbols)
}

fn take_identifier(text: &str) -> &str {
    let mut end = 0usize;
    for (idx, ch) in text.char_indices() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            end = idx + ch.len_utf8();
        } else {
            break;
        }
    }
    &text[..end]
}

fn render_context_for_summary(candidates: &[QueryCandidate]) -> String {
    let mut blocks = Vec::new();
    let mut total_chars = 0usize;

    for (idx, candidate) in candidates
        .iter()
        .take(SUMMARY_MAX_CANDIDATES)
        .enumerate()
    {
        let snippet = build_summary_snippet(&candidate.chunk.content);
        let mut block = format!(
            "片段 {} | 文件: {} | 行号: {}-{} | 关键词得分: {:.4} | 命中原因: {}",
            idx + 1,
            candidate.chunk.path,
            candidate.chunk.start_line,
            candidate.chunk.end_line,
            candidate.score,
            render_candidate_reason_summary(candidate)
        );
        if !candidate.matched_terms.is_empty() {
            block.push_str(&format!(
                "\n命中词: {}",
                candidate.matched_terms.join(", ")
            ));
        }
        if !candidate.matched_phrases.is_empty() {
            block.push_str(&format!(
                "\n命中短语: {}",
                candidate.matched_phrases.join(" | ")
            ));
        }
        block.push_str(&format!("\n代码片段:\n{}\n", snippet));

        if total_chars > 0 && total_chars + block.chars().count() > SUMMARY_CONTEXT_MAX_CHARS {
            break;
        }
        total_chars += block.chars().count();
        blocks.push(block);
    }

    blocks.join("\n")
}

fn render_context_for_rerank(candidates: &[RerankCandidate]) -> String {
    let mut blocks = Vec::new();
    let mut total_chars = 0usize;

    for candidate in candidates {
        let block = format!(
            "候选ID: {}\n文件: {}\n行号: {}-{}\n命中原因: {}\n代码片段:\n{}\n",
            candidate.id,
            candidate.path,
            candidate.start_line,
            candidate.end_line,
            candidate.reason_summary,
            candidate.snippet
        );
        if total_chars > 0 && total_chars + block.chars().count() > RERANK_CONTEXT_MAX_CHARS {
            break;
        }
        total_chars += block.chars().count();
        blocks.push(block);
    }

    blocks.join("\n")
}

fn render_structured_fallback(
    query: &str,
    candidates: &[QueryCandidate],
    fallback_reason: Option<&str>,
) -> String {
    let mut lines = Vec::new();
    lines.push(
        "Conclusion: 已召回最相关的本地代码片段，当前返回结构化本地结果。".to_string(),
    );
    if let Some(reason) = fallback_reason {
        lines.push(format!("Fallback reason: {}", reason));
    }
    lines.push(format!("Question: {}", query.trim()));
    lines.push(String::new());
    lines.push("Key files and why:".to_string());

    for (idx, candidate) in candidates.iter().enumerate() {
        lines.push(format!(
            "{}. {}:{}-{}",
            idx + 1,
            candidate.chunk.path,
            candidate.chunk.start_line,
            candidate.chunk.end_line
        ));
        lines.push(format!(
            "   Reasons: {}",
            render_candidate_reason_summary(candidate)
        ));
        if !candidate.matched_terms.is_empty() {
            lines.push(format!(
                "   Matched terms: {}",
                candidate.matched_terms.join(", ")
            ));
        }
        if !candidate.matched_phrases.is_empty() {
            lines.push(format!(
                "   Matched phrases: {}",
                candidate.matched_phrases.join(" | ")
            ));
        }
        lines.push(format!(
            "   Snippet: {}",
            summarize_snippet(&candidate.chunk.content)
        ));
    }

    lines.join("\n")
}

fn summarize_snippet(content: &str) -> String {
    let lines = content
        .lines()
        .take(8)
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        "(empty snippet)".to_string()
    } else {
        lines.join(" ⏐ ")
    }
}

fn build_summary_snippet(content: &str) -> String {
    let lines = content
        .lines()
        .map(|line| line.trim_end())
        .filter(|line| !line.trim().is_empty())
        .take(SUMMARY_SNIPPET_MAX_LINES)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return "(empty snippet)".to_string();
    }

    let joined = lines.join("\n");
    if joined.chars().count() <= SUMMARY_SNIPPET_MAX_CHARS {
        return joined;
    }

    let mut shortened = joined
        .chars()
        .take(SUMMARY_SNIPPET_MAX_CHARS)
        .collect::<String>();
    shortened.push_str("\n... [truncated]");
    shortened
}

fn render_candidate_reason_summary(candidate: &QueryCandidate) -> String {
    let mut parts = Vec::new();
    if !candidate.reason_labels.is_empty() {
        parts.push(candidate.reason_labels.join(" / "));
    }
    if !candidate.matched_terms.is_empty() {
        parts.push(format!("terms={}", candidate.matched_terms.join(", ")));
    }
    if !candidate.matched_phrases.is_empty() {
        parts.push(format!("phrases={}", candidate.matched_phrases.join(" | ")));
    }
    parts.join("; ")
}

fn build_local_summary_payload(model: &str, query: &str, context: &str) -> Value {
    json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": "你是本地代码检索助手。请基于给定代码片段回答用户问题。要求：1. 只基于提供的片段回答，不要杜撰。2. 先给结论，再列出关键文件与原因。3. 尽量保留文件路径和行号范围。4. 如果信息不足，明确说明。"
            },
            {
                "role": "user",
                "content": format!("用户问题：\n{}\n\n召回代码片段：\n{}", query.trim(), context)
            }
        ],
        "temperature": 0.1,
        "reasoning_effort": local_summary_reasoning_effort_for_model(model)
    })
}

fn build_local_rerank_payload(model: &str, query: &str, context: &str) -> Value {
    json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": "你是本地代码检索重排助手。请只根据给定候选代码片段，按与用户问题的相关性从高到低重新排序。只输出 JSON，对象格式必须是 {\"ordered_ids\": [\"候选ID1\", \"候选ID2\"]}。不要输出解释，不要输出 markdown，不要补充不存在的 ID。"
            },
            {
                "role": "user",
                "content": format!("用户问题：\n{}\n\n候选代码片段：\n{}", query.trim(), context)
            }
        ],
        "temperature": 0.0,
        "response_format": {
            "type": "json_object"
        }
    })
}

fn local_summary_reasoning_effort_for_model(model: &str) -> &'static str {
    let normalized = model.trim().to_ascii_lowercase();
    if normalized == "gpt-5-pro" {
        "high"
    } else {
        "low"
    }
}

fn parse_chat_completion_text(response_text: &str) -> Result<String, String> {
    let value: Value =
        serde_json::from_str(response_text).map_err(|e| format!("Invalid summary JSON: {e}"))?;
    let choices = value
        .get("choices")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Summary response missing choices.".to_string())?;
    let first = choices
        .first()
        .ok_or_else(|| "Summary response returned no choices.".to_string())?;
    let message = first
        .get("message")
        .ok_or_else(|| "Summary response missing message.".to_string())?;

    if let Some(content) = message.get("content").and_then(|v| v.as_str()) {
        return Ok(content.trim().to_string());
    }

    let parts = message
        .get("content")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Summary response missing content.".to_string())?;
    let text = parts
        .iter()
        .filter_map(|part| part.get("text").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if text.is_empty() {
        Err("Summary response returned empty content.".to_string())
    } else {
        Ok(text)
    }
}

fn parse_rerank_result(
    response_text: &str,
    candidates: &[RerankCandidate],
) -> Result<Vec<String>, String> {
    let text = parse_chat_completion_text(response_text)?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| format!("Invalid rerank JSON: {e}"))?;
    let ordered_ids = value
        .get("ordered_ids")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Rerank response missing ordered_ids.".to_string())?;
    let valid_ids = candidates
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect::<HashSet<_>>();
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for id in ordered_ids.iter().filter_map(|value| value.as_str()) {
        if valid_ids.contains(id) && seen.insert(id.to_string()) {
            result.push(id.to_string());
        }
    }
    if result.is_empty() {
        return Err("Rerank response returned no valid candidate ids.".to_string());
    }
    Ok(result)
}

fn sanitize_response_text(text: &str) -> String {
    let compact = text.replace("\r\n", " ").replace('\n', " ");
    let trimmed = compact.trim();
    if trimmed.chars().count() > 400 {
        trimmed.chars().take(400).collect::<String>()
    } else {
        trimmed.to_string()
    }
}

fn build_query_cache_key(query: &str, candidate_signature: &str, summary_mode: &str) -> String {
    sha256_hex(
        format!(
            "{}|{}|{}",
            query.trim(),
            candidate_signature.trim(),
            summary_mode
        )
        .as_bytes(),
    )
}

fn build_rerank_cache_key(
    query: &str,
    candidate_signature: &str,
    rerank_mode: &str,
    rerank_model: &str,
) -> String {
    sha256_hex(
        format!(
            "{}|{}|{}|{}",
            query.trim(),
            candidate_signature.trim(),
            rerank_mode,
            rerank_model
        )
        .as_bytes(),
    )
}

fn determine_rebuild_reason(
    index_rebuild_mode: LocalIndexRebuildMode,
    had_manifest: bool,
    recovery_reason: Option<&str>,
) -> String {
    if let Some(reason) = recovery_reason {
        return reason.to_string();
    }
    if index_rebuild_mode == LocalIndexRebuildMode::ForceFull {
        return "config_force_full".to_string();
    }
    if had_manifest {
        "incremental_refresh".to_string()
    } else {
        "initial_build".to_string()
    }
}

fn is_query_cache_entry_expired(entry: &QueryCacheEntry, now_unix_sec: u64) -> bool {
    now_unix_sec.saturating_sub(entry.cached_at_unix_sec) > QUERY_CACHE_TTL_SEC
}

fn build_candidate_signature(candidates: &[QueryCandidate]) -> String {
    let mut hasher = Sha256::new();
    for candidate in candidates {
        hasher.update(candidate.chunk.id.as_bytes());
        hasher.update(candidate.chunk.content_hash.as_bytes());
        hasher.update(candidate.chunk.start_line.to_string().as_bytes());
        hasher.update(candidate.chunk.end_line.to_string().as_bytes());
    }
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

fn apply_rerank_order(candidates: &[QueryCandidate], ordered_ids: &[String]) -> Vec<QueryCandidate> {
    let mut by_id = candidates
        .iter()
        .cloned()
        .map(|candidate| (candidate.chunk.id.clone(), candidate))
        .collect::<HashMap<_, _>>();
    let mut ordered = Vec::new();
    for id in ordered_ids {
        if let Some(candidate) = by_id.remove(id) {
            ordered.push(candidate);
        }
    }
    let mut remaining = by_id.into_values().collect::<Vec<_>>();
    remaining.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
    });
    ordered.extend(remaining);
    ordered
}

fn build_index_signature(entries: &[LocalFileManifestEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.path.as_bytes());
        hasher.update(entry.file_hash.as_bytes());
        hasher.update(entry.chunk_count.to_string().as_bytes());
    }
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

fn merge_string_vec(target: &mut Vec<String>, incoming: &[String]) {
    let mut set = target.iter().cloned().collect::<HashSet<_>>();
    for item in incoming {
        if set.insert(item.clone()) {
            target.push(item.clone());
        }
    }
    target.sort();
}

fn sort_string_set(set: HashSet<String>) -> Vec<String> {
    let mut values = set.into_iter().collect::<Vec<_>>();
    values.sort();
    values
}

fn merge_chunk_content(left: &str, right: &str) -> String {
    if left.trim().is_empty() {
        return right.to_string();
    }
    if right.trim().is_empty() {
        return left.to_string();
    }
    format!("{}\n...\n{}", left.trim_end(), right.trim_start())
}

fn shorten_hash(value: &str) -> String {
    value.chars().take(12).collect()
}

fn system_time_to_unix_seconds(value: SystemTime) -> u64 {
    value
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn now_unix_seconds() -> u64 {
    system_time_to_unix_seconds(SystemTime::now())
}

fn contains_cjk(text: &str) -> bool {
    text.chars().any(|ch| ('\u{4e00}'..='\u{9fff}').contains(&ch))
}

fn sha256_hex(input: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::{
        LocalChunkRecord, LocalIndexRebuildMode, LocalRerankMode, QueryCacheEntry,
        apply_rerank_order, build_local_summary_payload, build_query_terms,
        build_rerank_candidate, build_summary_snippet, build_token_freq,
        compute_local_rerank_timeout, compute_local_summary_timeout, determine_rebuild_reason,
        is_query_cache_entry_expired, local_summary_reasoning_effort_for_model,
        merge_adjacent_candidates, normalize_search_text, parse_rerank_result, prepare_query,
        rank_candidates, render_context_for_rerank, render_context_for_summary,
        render_structured_fallback, split_content_into_local_chunks, tokenize_search_text,
    };
    use std::time::Duration;

    fn chunk(path: &str, content: &str, symbols: &[&str]) -> LocalChunkRecord {
        chunk_with_range(path, 1, 10, content, symbols)
    }

    fn chunk_with_range(
        path: &str,
        start_line: usize,
        end_line: usize,
        content: &str,
        symbols: &[&str],
    ) -> LocalChunkRecord {
        let normalized = super::normalize_search_text(&format!("{} {}", path, content));
        let token_freq = build_token_freq(&normalized);
        let token_count = token_freq.values().map(|v| *v as usize).sum::<usize>();
        LocalChunkRecord {
            id: path.to_string(),
            path: path.to_string(),
            file_name: path.rsplit('/').next().unwrap_or(path).to_string(),
            start_line,
            end_line,
            content_hash: "hash".to_string(),
            content: content.to_string(),
            normalized_text: normalized,
            token_freq,
            token_count,
            symbol_tokens: symbols.iter().map(|item| item.to_string()).collect(),
        }
    }

    #[test]
    fn chunk_split_preserves_line_ranges() {
        let content = (1..=260)
            .map(|n| format!("line {}", n))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = split_content_into_local_chunks("src/main.rs", &content, 800);
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].0, 1);
        assert!(chunks[0].1 >= 100);
        assert!(chunks[1].0 > 1);
    }

    #[test]
    fn chunk_split_respects_max_lines_per_blob() {
        let content = (1..=75)
            .map(|n| format!("line {}", n))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = split_content_into_local_chunks("src/main.rs", &content, 30);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|(_, end, text)| {
            let line_count = text.lines().count();
            line_count <= 30 && *end >= line_count
        }));
    }

    #[test]
    fn query_terms_cover_english_and_cjk() {
        let terms = build_query_terms("search_context 本地 检索");
        assert!(terms.weighted_terms.iter().any(|t| t.term.contains("本地")));
        assert!(terms.weighted_terms.iter().any(|t| t.term.contains("search")));
    }

    #[test]
    fn tokenize_search_text_splits_camel_and_snake_case() {
        let tokens = tokenize_search_text("SearchProviderKind local_search searchContext");
        assert!(tokens.contains(&"search".to_string()));
        assert!(tokens.contains(&"provider".to_string()));
        assert!(tokens.contains(&"kind".to_string()));
        assert!(tokens.contains(&"local".to_string()));
        assert!(tokens.contains(&"context".to_string()));
    }

    #[test]
    fn rank_candidates_prefers_filename_and_symbol_hits() {
        let query = build_query_terms("LocalSearchProvider search_context");
        let chunks = vec![
            chunk(
                "src/index/local_search.rs",
                "impl LocalSearchProvider { fn search_context(&self) {} }",
                &["local", "search", "provider", "context"],
            ),
            chunk("src/ui/window.rs", "renders the prompt window", &["window"]),
        ];
        let ranked = rank_candidates(&query, chunks, 1);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].chunk.path, "src/index/local_search.rs");
        assert!(ranked[0].reason_labels.iter().any(|v| v == "符号命中"));
    }

    #[test]
    fn normalize_search_text_keeps_cjk_and_lowercases_ascii() {
        let normalized = normalize_search_text("SearchContext 本地检索");
        assert!(normalized.contains("search context"));
        assert!(normalized.contains("本地检索"));
    }

    #[test]
    fn structured_fallback_contains_reason_blocks() {
        let query = "where is search_context";
        let candidates = rank_candidates(
            &build_query_terms("search_context local_search"),
            vec![chunk(
                "src/index/local_search.rs",
                "fn search_context() { /* ... */ }",
                &["search", "context", "local"],
            )],
            1,
        );
        let text = render_structured_fallback(query, &candidates, Some("summary_mode=gpt"));
        assert!(text.contains("Conclusion:"));
        assert!(text.contains("Reasons:"));
        assert!(text.contains("Matched terms:"));
    }

    #[test]
    fn summary_snippet_is_trimmed_to_manageable_size() {
        let content = (1..=60)
            .map(|n| format!("line {} {}", n, "x".repeat(80)))
            .collect::<Vec<_>>()
            .join("\n");
        let snippet = build_summary_snippet(&content);
        assert!(snippet.lines().count() <= super::SUMMARY_SNIPPET_MAX_LINES + 1);
        assert!(snippet.contains("truncated") || snippet.chars().count() <= super::SUMMARY_SNIPPET_MAX_CHARS);
    }

    #[test]
    fn summary_context_limits_candidate_volume() {
        let candidates = (0..8)
            .map(|idx| super::QueryCandidate {
                score: 10.0 - idx as f32,
                chunk: chunk_with_range(
                    &format!("src/file_{}.rs", idx),
                    1,
                    40,
                    &(1..=30)
                        .map(|n| format!("line {} {}", n, "y".repeat(90)))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    &["local", "search", "context"],
                ),
                matched_terms: vec!["local".to_string(), "search".to_string()],
                matched_phrases: vec!["local search".to_string()],
                reason_labels: vec!["正文命中".to_string()],
            })
            .collect::<Vec<_>>();
        let context = render_context_for_summary(&candidates);
        assert!(context.chars().count() <= super::SUMMARY_CONTEXT_MAX_CHARS);
        assert!(context.matches("片段 ").count() <= super::SUMMARY_MAX_CANDIDATES);
    }

    #[test]
    fn token_freq_counts_repeated_terms() {
        let freq = build_token_freq("search context search local");
        assert_eq!(freq.get("search"), Some(&2));
        assert_eq!(freq.get("local"), Some(&1));
    }

    #[test]
    fn merge_adjacent_candidates_applies_soft_file_diversity() {
        let query = build_query_terms("search_context local");
        let ranked = rank_candidates(
            &query,
            vec![
                chunk_with_range(
                    "src/index/local_search.rs",
                    1,
                    10,
                    "fn search_context() { let local = true; }",
                    &["search", "context", "local"],
                ),
                chunk_with_range(
                    "src/index/local_search.rs",
                    80,
                    90,
                    "fn search_context_cached() { let local = true; }",
                    &["search", "context", "local"],
                ),
                chunk_with_range(
                    "src/index/local_search.rs",
                    160,
                    170,
                    "fn search_context_ranked() { let local = true; }",
                    &["search", "context", "local"],
                ),
                chunk_with_range(
                    "src/main.rs",
                    20,
                    30,
                    "fn search_context_entry() { let local = true; }",
                    &["search", "context", "local"],
                ),
            ],
            4,
        );
        let merged = merge_adjacent_candidates(ranked, 3);
        assert_eq!(merged.len(), 3);
        assert!(merged.iter().any(|item| item.chunk.path == "src/main.rs"));
    }

    #[test]
    fn determine_rebuild_reason_prefers_recovery_reason() {
        let reason = determine_rebuild_reason(
            LocalIndexRebuildMode::Auto,
            true,
            Some("chunk_unreadable"),
        );
        assert_eq!(reason, "chunk_unreadable");
    }

    #[test]
    fn determine_rebuild_reason_distinguishes_initial_build() {
        let reason = determine_rebuild_reason(LocalIndexRebuildMode::Auto, false, None);
        assert_eq!(reason, "initial_build");
    }

    #[test]
    fn query_cache_entry_expires_after_ttl() {
        let entry = QueryCacheEntry {
            query: "search_context".to_string(),
            candidate_signature: "sig".to_string(),
            summary_mode: "local_fallback_only".to_string(),
            result: "result".to_string(),
            cached_at_unix_sec: 10,
        };
        assert!(is_query_cache_entry_expired(
            &entry,
            10 + super::QUERY_CACHE_TTL_SEC + 1
        ));
        assert!(!is_query_cache_entry_expired(
            &entry,
            10 + super::QUERY_CACHE_TTL_SEC
        ));
    }

    #[test]
    fn local_summary_payload_uses_configured_model_and_reasoning_effort() {
        let payload = build_local_summary_payload("gpt-5.4", "where", "snippet");
        assert_eq!(payload.get("model").and_then(|v| v.as_str()), Some("gpt-5.4"));
        assert_eq!(
            payload.get("reasoning_effort").and_then(|v| v.as_str()),
            Some("low")
        );
    }

    #[test]
    fn local_summary_reasoning_effort_handles_gpt_5_pro_exception() {
        assert_eq!(local_summary_reasoning_effort_for_model("gpt-5-pro"), "high");
        assert_eq!(local_summary_reasoning_effort_for_model("gpt-5.4"), "low");
    }

    #[test]
    fn compute_local_rerank_timeout_skips_when_budget_is_too_low() {
        let timeout = compute_local_rerank_timeout(50, Duration::from_secs(41), 10);
        assert!(timeout.is_none());
    }

    #[test]
    fn compute_local_rerank_timeout_caps_by_configured_timeout() {
        let timeout =
            compute_local_rerank_timeout(50, Duration::from_secs(2), 12).expect("rerank budget");
        assert_eq!(timeout, Duration::from_secs(12));
    }

    #[test]
    fn compute_local_rerank_timeout_enforces_minimum_default_floor() {
        let timeout =
            compute_local_rerank_timeout(50, Duration::from_secs(2), 3).expect("rerank budget");
        assert_eq!(timeout, Duration::from_secs(super::DEFAULT_RERANK_TIMEOUT_SEC));
    }

    #[test]
    fn parse_rerank_result_keeps_valid_unique_ids_only() {
        let candidates = vec![
            build_rerank_candidate(&super::QueryCandidate {
                score: 3.0,
                chunk: chunk("src/a.rs", "fn alpha() {}", &["alpha"]),
                matched_terms: vec!["alpha".to_string()],
                matched_phrases: Vec::new(),
                reason_labels: vec!["正文命中".to_string()],
            }),
            build_rerank_candidate(&super::QueryCandidate {
                score: 2.0,
                chunk: chunk("src/b.rs", "fn beta() {}", &["beta"]),
                matched_terms: vec!["beta".to_string()],
                matched_phrases: Vec::new(),
                reason_labels: vec!["正文命中".to_string()],
            }),
        ];
        let response = serde_json::json!({
            "choices": [{
                "message": {
                    "content": serde_json::json!({
                        "ordered_ids": [
                            "src/b.rs",
                            "src/b.rs",
                            "missing.rs",
                            "src/a.rs"
                        ]
                    }).to_string()
                }
            }]
        })
        .to_string();

        let ordered = parse_rerank_result(&response, &candidates).expect("parse rerank result");
        assert_eq!(
            ordered,
            vec!["src/b.rs".to_string(), "src/a.rs".to_string()]
        );
    }

    #[test]
    fn apply_rerank_order_reorders_and_preserves_remaining_score_order() {
        let candidates = vec![
            super::QueryCandidate {
                score: 9.0,
                chunk: chunk("src/a.rs", "fn alpha() {}", &["alpha"]),
                matched_terms: vec!["alpha".to_string()],
                matched_phrases: Vec::new(),
                reason_labels: vec!["正文命中".to_string()],
            },
            super::QueryCandidate {
                score: 7.0,
                chunk: chunk("src/b.rs", "fn beta() {}", &["beta"]),
                matched_terms: vec!["beta".to_string()],
                matched_phrases: Vec::new(),
                reason_labels: vec!["正文命中".to_string()],
            },
            super::QueryCandidate {
                score: 8.0,
                chunk: chunk("src/c.rs", "fn gamma() {}", &["gamma"]),
                matched_terms: vec!["gamma".to_string()],
                matched_phrases: Vec::new(),
                reason_labels: vec!["正文命中".to_string()],
            },
        ];

        let ordered = apply_rerank_order(
            &candidates,
            &["src/b.rs".to_string(), "src/a.rs".to_string()],
        );
        let ordered_paths = ordered
            .iter()
            .map(|candidate| candidate.chunk.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ordered_paths, vec!["src/b.rs", "src/a.rs", "src/c.rs"]);
    }

    #[test]
    fn render_context_for_rerank_respects_context_budget() {
        let candidates = (0..12)
            .map(|idx| build_rerank_candidate(&super::QueryCandidate {
                score: 20.0 - idx as f32,
                chunk: chunk_with_range(
                    &format!("src/rerank_{}.rs", idx),
                    1,
                    60,
                    &(1..=30)
                        .map(|line| format!("line {} {}", line, "z".repeat(110)))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    &["rerank", "local", "search"],
                ),
                matched_terms: vec!["rerank".to_string(), "search".to_string()],
                matched_phrases: vec!["local search".to_string()],
                reason_labels: vec!["正文命中".to_string()],
            }))
            .collect::<Vec<_>>();
        let context = render_context_for_rerank(&candidates);
        assert!(context.chars().count() <= super::RERANK_CONTEXT_MAX_CHARS);
        assert!(context.contains("候选ID:"));
    }

    #[test]
    fn prepare_query_expands_broad_config_query_with_code_hints() {
        let prepared = prepare_query("codex 的配置优化");
        assert!(prepared.broad_intent);
        assert!(prepared.hint_terms.iter().any(|term| term == "codex"));
        assert!(prepared.hint_terms.iter().any(|term| term == "provider"));
        assert!(
            prepared
                .terms
                .weighted_terms
                .iter()
                .any(|term| term.term == "timeout")
        );
    }

    #[test]
    fn prepare_query_keeps_specific_code_query_non_broad() {
        let prepared = prepare_query("LocalSearchProvider search_context timeout");
        assert!(!prepared.broad_intent);
        assert!(prepared.terms.weighted_terms.iter().any(|term| term.term == "search"));
    }

    #[test]
    fn rerank_mode_broad_only_only_triggers_for_broad_queries() {
        let broad = prepare_query("codex 的配置优化");
        let specific = prepare_query("LocalSearchProvider search_context timeout");
        let candidates = vec![
            super::QueryCandidate {
                score: 9.0,
                chunk: chunk("src/a.rs", "fn a() {}", &["a1"]),
                matched_terms: vec!["local".to_string()],
                matched_phrases: Vec::new(),
                reason_labels: vec!["正文命中".to_string()],
            },
            super::QueryCandidate {
                score: 8.0,
                chunk: chunk("src/b.rs", "fn b() {}", &["b1"]),
                matched_terms: vec!["search".to_string()],
                matched_phrases: Vec::new(),
                reason_labels: vec!["正文命中".to_string()],
            },
            super::QueryCandidate {
                score: 7.0,
                chunk: chunk("src/c.rs", "fn c() {}", &["c1"]),
                matched_terms: vec!["context".to_string()],
                matched_phrases: Vec::new(),
                reason_labels: vec!["正文命中".to_string()],
            },
            super::QueryCandidate {
                score: 6.0,
                chunk: chunk("src/d.rs", "fn d() {}", &["d1"]),
                matched_terms: vec!["provider".to_string()],
                matched_phrases: Vec::new(),
                reason_labels: vec!["正文命中".to_string()],
            },
        ];
        let provider = super::LocalSearchProvider {
            project_root: std::env::temp_dir(),
            storage_dir: std::env::temp_dir().join("storage"),
            chunks_dir: std::env::temp_dir().join("chunks"),
            query_cache_dir: std::env::temp_dir().join("query-cache"),
            rerank_cache_dir: std::env::temp_dir().join("rerank-cache"),
            text_extensions: Default::default(),
            exclude_patterns: Vec::new(),
            max_lines_per_blob: 800,
            codex_api_base: "https://example.com/v1".to_string(),
            codex_model: "gpt-5.4-mini".to_string(),
            rerank_model: "gpt-5.4-mini".to_string(),
            summary_mode: super::LocalSummaryMode::LocalFallbackOnly,
            rerank_mode: LocalRerankMode::BroadOnly,
            index_rebuild_mode: LocalIndexRebuildMode::Auto,
            search_timeout_sec: 50,
            rerank_pool_size: 12,
            rerank_timeout_sec: 10,
            client: None,
        };

        assert!(provider.should_use_rerank(&broad, &candidates));
        assert!(!provider.should_use_rerank(&specific, &candidates));
        assert!(!provider.should_use_rerank(&broad, &candidates[..3]));
    }

    #[test]
    fn compute_local_summary_timeout_skips_when_budget_is_too_low() {
        let timeout = compute_local_summary_timeout(50, Duration::from_secs(43));
        assert!(timeout.is_none());
    }

    #[test]
    fn compute_local_summary_timeout_caps_soft_budget() {
        let timeout =
            compute_local_summary_timeout(50, Duration::from_secs(5)).expect("summary budget");
        assert_eq!(timeout, Duration::from_secs(40));
    }
}
