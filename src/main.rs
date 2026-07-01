mod config;
mod enhancer;
mod index;
mod logging;
mod mcp;
mod ui;
mod utils;

use config::Config;
use enhancer::codex_provider::CodexProvider;
use enhancer::enhancer::RemoteProvider;
use enhancer::provider::{EnhanceProvider, EnhanceProviderKind};
use index::{LocalIndexRebuildMode, LocalRerankMode, LocalSummaryMode, SearchProviderKind};
use index::local_search::LocalSearchProvider;
use index::manager::IndexManager;
use logging::{LogLevel, init_mcp_logger};
use mcp::{McpLogger, McpServer, ToolHandler, log_debug, schemas};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::time::timeout;
use ui::session::{ContinueCallback, SessionAction, is_headless_mode, run_prompt_session};

/// 程序入口：初始化配置、工具列表与 MCP 服务端。
fn main() -> std::io::Result<()> {
    let config = match config::init_config() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };

    let tools = schemas::tool_list();
    let runtime = Arc::new(Runtime::new().expect("Failed to create Tokio runtime"));
    let config = Arc::new(config);

    let handler: ToolHandler = Arc::new({
        let runtime = runtime.clone();
        let config = config.clone();
        move |name, args| handle_tool_call(name, args, &config, &runtime)
    });

    let server = McpServer::new(tools, handler);
    let logger = server.logger();
    init_mcp_logger(build_mcp_sender(logger));

    server.run()
}

/// MCP 工具分发器，根据工具名路由到具体处理函数。
fn handle_tool_call(
    name: &str,
    args: Option<serde_json::Value>,
    config: &Arc<Config>,
    runtime: &Arc<Runtime>,
) -> Result<String, String> {
    match name {
        "search_context" => handle_search_context(args, config, runtime),
        "enhance_prompt" => handle_enhance_prompt(args, config, runtime),
        _ => Err(format!("Unknown tool: {name}")),
    }
}

/// `search_context` 的处理入口。
///
/// 负责校验参数、准备索引管理器并执行检索。
fn handle_search_context(
    args: Option<serde_json::Value>,
    config: &Arc<Config>,
    runtime: &Arc<Runtime>,
) -> Result<String, String> {
    let started = Instant::now();
    let args = args.unwrap_or_else(|| serde_json::json!({}));
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
    if query.is_empty() {
        return Ok("Error: query is required".to_string());
    }

    let project_root_path = args
        .get("project_root_path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if project_root_path.is_empty() {
        return Ok("Error: project_root_path is required.".to_string());
    }

    let project_root = PathBuf::from(project_root_path);
    if !project_root.exists() {
        return Ok(format!(
            "Error: Project path does not exist: {}",
            project_root_path
        ));
    }
    if !project_root.is_dir() {
        return Ok(format!(
            "Error: Project path is not a directory: {}",
            project_root_path
        ));
    }

    if config.enable_log {
        let _ = logging::enable_file_log(&project_root);
    }

    let search_provider_kind = resolve_search_provider_kind(config)?;

    let timeout_sec = std::env::var("ACE_TOOL_SEARCH_TIMEOUT_SEC")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v >= 10 && *v <= 300)
        .unwrap_or(50);
    log_debug(format!(
        "search_context: start root={} resolved_root={} query_len={} timeout={}s provider={}",
        project_root_path,
        project_root.display(),
        query.chars().count(),
        timeout_sec,
        search_provider_kind.as_str()
    ));

    let result = runtime.block_on(async {
        timeout(Duration::from_secs(timeout_sec), async {
            match search_provider_kind {
                SearchProviderKind::Remote => {
                    let manager = IndexManager::new(
                        project_root.clone(),
                        config.base_url.clone(),
                        config.token.clone(),
                        config.text_extensions.clone(),
                        config.max_lines_per_blob,
                        config.exclude_patterns.clone(),
                    )
                    .map_err(|e| format!("Error: {e}"))?;
                    Ok::<String, String>(manager.search_context(query).await)
                }
                SearchProviderKind::Local => {
                    let local_summary_mode = resolve_local_summary_mode(config)?;
                    let local_rerank_mode = resolve_local_rerank_mode(config)?;
                    let local_index_rebuild_mode = resolve_local_index_rebuild_mode(config)?;
                    let provider = LocalSearchProvider::new(
                        project_root.clone(),
                        config.text_extensions.clone(),
                        config.max_lines_per_blob,
                        config.exclude_patterns.clone(),
                        config.codex_api_base.clone(),
                        config.codex_api_key.clone(),
                        config.codex_model.clone(),
                        local_summary_mode,
                        local_rerank_mode,
                        local_index_rebuild_mode,
                        timeout_sec,
                        config.local_rerank_pool_size,
                        config.local_rerank_timeout_sec,
                        config.local_rerank_model.clone(),
                    )
                    .map_err(|e| format!("Error: {e}"))?;
                    provider.search_context(query).await
                }
            }
        })
        .await
    });
    match result {
        Ok(Ok(text)) => {
            log_debug(format!(
                "search_context: done elapsed={}ms",
                started.elapsed().as_millis()
            ));
            Ok(text)
        }
        Ok(Err(err)) => Ok(err),
        Err(_) => {
            log_debug(format!(
                "search_context: timeout elapsed={}ms",
                started.elapsed().as_millis()
            ));
            Ok(format!(
                "Error: search_context timed out after {} seconds. Narrow query or reduce project scope and retry.",
                timeout_sec
            ))
        }
    }
}

/// `enhance_prompt` 的处理入口。
///
/// 负责校验参数、调用增强服务，并通过 UI 让用户确认返回内容。
fn handle_enhance_prompt(
    args: Option<serde_json::Value>,
    config: &Arc<Config>,
    runtime: &Arc<Runtime>,
) -> Result<String, String> {
    let started = Instant::now();
    log_debug("enhance_prompt: start".to_string());
    let args = args.unwrap_or_else(|| serde_json::json!({}));
    let raw_prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    let history = args
        .get("conversation_history")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let prompt = match resolve_effective_prompt(raw_prompt, history) {
        Some(value) => value,
        None => {
            log_debug(
                "enhance_prompt: empty prompt after cleanup and history fallback; skip enhancement"
                    .to_string(),
            );
            return Ok(raw_prompt.trim().to_string());
        }
    };
    if history.is_empty() {
        log_debug(
            "enhance_prompt: conversation_history is empty, using prompt-only enhancement"
                .to_string(),
        );
    }

    let provider_kind = resolve_provider_kind(&args, config)?;
    let enhance_timeout_sec = resolve_enhance_timeout_sec(config, provider_kind);
    log_debug(format!(
        "enhance_prompt: provider={} prompt_len={} history_len={} timeout={}s",
        provider_kind.as_str(),
        prompt.chars().count(),
        history.chars().count(),
        enhance_timeout_sec
    ));

    let project_root_path = args
        .get("project_root_path")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let dedupe_ttl = Duration::from_secs(180);
    let cache_key = build_enhance_cache_key(&project_root_path, provider_kind, &prompt, history);
    if let Some(cached) = lookup_recent_enhance_result(cache_key, dedupe_ttl) {
        log_debug(format!(
            "enhance_prompt: duplicate call suppressed ttl={}s key={}",
            dedupe_ttl.as_secs(),
            cache_key
        ));
        return Ok(cached);
    }

    let provider: Arc<dyn EnhanceProvider> = match provider_kind {
        EnhanceProviderKind::Remote => {
            log_debug("enhance_prompt: building index manager".to_string());
            let manager = IndexManager::new(
                project_root_path,
                config.base_url.clone(),
                config.token.clone(),
                config.text_extensions.clone(),
                config.max_lines_per_blob,
                config.exclude_patterns.clone(),
            )
            .map_err(|e| format!("Error: {e}"))?;

            Arc::new(
                RemoteProvider::new(manager, config.base_url.clone(), config.token.clone())
                    .map_err(|e| format!("Error: {e}"))?,
            )
        }
        EnhanceProviderKind::Codex => {
            log_debug(format!(
                "enhance_prompt: create codex provider api_base={} model={} timeout={}s",
                config.codex_api_base, config.codex_model, enhance_timeout_sec
            ));
            Arc::new(
                CodexProvider::new(
                    config.codex_api_base.clone(),
                    config.codex_api_key.clone(),
                    config.codex_model.clone(),
                    enhance_timeout_sec,
                )
                .map_err(|e| format!("Error: {e}"))?,
            )
        }
    };
    log_debug(format!(
        "enhance_prompt: active_provider={}",
        provider.kind().as_str()
    ));

    // UI 等待时间主要用于用户交互，接口超时用于避免网络请求卡死。
    let ui_timeout = Duration::from_secs(config.ui_timeout_sec);
    let enhance_timeout = Duration::from_secs(enhance_timeout_sec);

    if is_headless_mode() {
        log_debug("enhance_prompt: headless mode enabled".to_string());
        let enhance_started = Instant::now();
        let current_prompt = match runtime
            .block_on(async { timeout(enhance_timeout, provider.enhance(&prompt, history)).await })
        {
            Ok(Ok(text)) => {
                log_debug(format!(
                    "enhance_prompt: provider done elapsed={}ms result_len={}",
                    enhance_started.elapsed().as_millis(),
                    text.chars().count()
                ));
                text
            }
            Ok(Err(err)) => {
                log_debug(format!(
                    "enhance_prompt: provider error elapsed={}ms kind={}",
                    enhance_started.elapsed().as_millis(),
                    classify_enhance_error(&err)
                ));
                if err.to_lowercase().contains("timeout") {
                    return Ok(format!(
                        "Enhancement timed out ({} seconds). Using original prompt: {}",
                        enhance_timeout_sec, prompt
                    ));
                }
                return Err(err);
            }
            Err(_) => {
                log_debug(format!(
                    "enhance_prompt: provider timeout elapsed={}ms",
                    enhance_started.elapsed().as_millis()
                ));
                return Ok(format!(
                    "Enhancement timed out ({} seconds). Using original prompt: {}",
                    enhance_timeout_sec, prompt
                ));
            }
        };

        let continue_cb: ContinueCallback = Arc::new(|current: String| Ok(current));
        log_debug("enhance_prompt: opening ui".to_string());
        let final_result = match run_prompt_session(&current_prompt, ui_timeout, continue_cb, false)
        {
            SessionAction::UseEnhanced(content) => {
                log_debug(format!(
                    "enhance_prompt: ui action=use_enhanced elapsed={}ms",
                    started.elapsed().as_millis()
                ));
                content
            }
            SessionAction::UseOriginal => {
                log_debug(format!(
                    "enhance_prompt: ui action=use_original elapsed={}ms",
                    started.elapsed().as_millis()
                ));
                prompt.clone()
            }
            SessionAction::EndConversation => {
                log_debug(format!(
                    "enhance_prompt: ui action=end_conversation elapsed={}ms",
                    started.elapsed().as_millis()
                ));
                "__END_CONVERSATION__".to_string()
            }
            SessionAction::Timeout => {
                log_debug(format!(
                    "enhance_prompt: ui action=timeout fallback=enhanced elapsed={}ms",
                    started.elapsed().as_millis()
                ));
                current_prompt.clone()
            }
        };
        store_recent_enhance_result(cache_key, &final_result);
        return Ok(final_result);
    }

    let provider = provider.clone();
    let runtime = runtime.clone();
    let history = history.to_string();
    let continue_provider_kind = provider.kind();
    let continue_cb: ContinueCallback = Arc::new(move |current: String| {
        log_debug("enhance_prompt: continue requested".to_string());
        let provider = provider.clone();
        let history = history.clone();
        let provider_kind = continue_provider_kind;
        let actual_kind = provider.kind();
        if actual_kind != provider_kind {
            return Err(format!(
                "Provider lock violated: expected {}, got {}",
                provider_kind.as_str(),
                actual_kind.as_str()
            ));
        }
        let (continue_prompt, continue_history) =
            prepare_continue_inputs(provider_kind, &current, &history);
        log_debug(format!(
            "enhance_prompt: continue payload provider={} prompt_len={} history_len={}",
            provider_kind.as_str(),
            continue_prompt.chars().count(),
            continue_history.chars().count()
        ));
        let continue_started = Instant::now();
        runtime.block_on(async move {
            match timeout(
                enhance_timeout,
                provider.enhance(&continue_prompt, &continue_history),
            )
            .await
            {
                Ok(Ok(text)) => {
                    log_debug(format!(
                        "enhance_prompt: continue done elapsed={}ms result_len={}",
                        continue_started.elapsed().as_millis(),
                        text.chars().count()
                    ));
                    Ok(text)
                }
                Ok(Err(err)) => {
                    log_debug(format!(
                        "enhance_prompt: continue error elapsed={}ms kind={}",
                        continue_started.elapsed().as_millis(),
                        classify_enhance_error(&err)
                    ));
                    Err(err)
                }
                Err(_) => {
                    log_debug(format!(
                        "enhance_prompt: continue timeout elapsed={}ms",
                        continue_started.elapsed().as_millis()
                    ));
                    Err(format!(
                        "Enhancement timed out ({} seconds).",
                        enhance_timeout.as_secs()
                    ))
                }
            }
        })
    });

    log_debug("enhance_prompt: opening ui".to_string());
    let final_result = match run_prompt_session(&prompt, ui_timeout, continue_cb, true) {
        SessionAction::UseEnhanced(content) => {
            log_debug(format!(
                "enhance_prompt: ui action=use_enhanced elapsed={}ms",
                started.elapsed().as_millis()
            ));
            content
        }
        SessionAction::UseOriginal => {
            log_debug(format!(
                "enhance_prompt: ui action=use_original elapsed={}ms",
                started.elapsed().as_millis()
            ));
            prompt.clone()
        }
        SessionAction::EndConversation => {
            log_debug(format!(
                "enhance_prompt: ui action=end_conversation elapsed={}ms",
                started.elapsed().as_millis()
            ));
            "__END_CONVERSATION__".to_string()
        }
        SessionAction::Timeout => {
            log_debug(format!(
                "enhance_prompt: ui action=timeout fallback=original elapsed={}ms",
                started.elapsed().as_millis()
            ));
            prompt.clone()
        }
    };
    store_recent_enhance_result(cache_key, &final_result);
    Ok(final_result)
}

fn prepare_continue_inputs(
    provider_kind: EnhanceProviderKind,
    current: &str,
    history: &str,
) -> (String, String) {
    if provider_kind != EnhanceProviderKind::Codex {
        return (current.to_string(), history.to_string());
    }

    // Codex 二次增强时，历史上下文会明显放大 token 规模，容易触发超时。
    // 续增强默认只用当前编辑框内容，不再做截断，避免信息量被压缩。
    (current.to_string(), String::new())
}

fn strip_enhance_markers(input: &str) -> String {
    fn is_boundary(ch: Option<char>) -> bool {
        ch.map(|c| !c.is_ascii_alphanumeric()).unwrap_or(true)
    }

    fn find_marker_len(input: &str, start: usize) -> Option<usize> {
        const MARKERS: [&str; 2] = ["-enhancer", "-enhance"];
        for marker in MARKERS {
            let end = start + marker.len();
            let slice = match input.get(start..end) {
                Some(s) => s,
                None => continue,
            };
            if !slice.eq_ignore_ascii_case(marker) {
                continue;
            }
            let before = input.get(..start).and_then(|s| s.chars().next_back());
            let after = input.get(end..).and_then(|s| s.chars().next());
            if is_boundary(before) && is_boundary(after) {
                return Some(marker.len());
            }
        }
        None
    }

    let mut out = String::with_capacity(input.len());
    let mut last = 0usize;
    for (idx, _) in input.char_indices() {
        if idx < last {
            continue;
        }
        let marker_len = match find_marker_len(input, idx) {
            Some(len) => len,
            None => continue,
        };
        if let Some(chunk) = input.get(last..idx) {
            out.push_str(chunk);
        }
        last = idx + marker_len;
    }
    if last >= input.len() {
        return out;
    }
    if let Some(rest) = input.get(last..) {
        out.push_str(rest);
    }
    out
}

fn resolve_effective_prompt(raw_prompt: &str, history: &str) -> Option<String> {
    let cleaned_prompt = strip_enhance_markers(raw_prompt);
    let prompt = cleaned_prompt.trim();
    if !prompt.is_empty() {
        return Some(prompt.to_string());
    }

    derive_prompt_from_history(history)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryRole {
    User,
    Assistant,
    Unknown,
}

fn derive_prompt_from_history(history: &str) -> Option<String> {
    let cleaned = strip_enhance_markers(history);
    let mut unknown_candidate: Option<String> = None;
    let mut assistant_candidate: Option<String> = None;

    for raw_line in cleaned.lines().rev() {
        let raw_line = raw_line.trim();
        if raw_line.is_empty() {
            continue;
        }
        let (role, line) = split_history_line(raw_line);
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case("__END_CONVERSATION__") {
            continue;
        }
        if !line.chars().any(is_prompt_char) {
            continue;
        }
        if is_enhance_status_line(line) {
            continue;
        }

        match role {
            HistoryRole::User => return Some(line.to_string()),
            HistoryRole::Unknown => {
                if unknown_candidate.is_none() {
                    unknown_candidate = Some(line.to_string());
                }
            }
            HistoryRole::Assistant => {
                if assistant_candidate.is_none() {
                    assistant_candidate = Some(line.to_string());
                }
            }
        }
    }

    unknown_candidate.or(assistant_candidate)
}

fn is_prompt_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ('\u{4E00}'..='\u{9FFF}').contains(&ch)
}

fn split_history_line(raw_line: &str) -> (HistoryRole, &str) {
    if let Some(rest) = strip_role_prefix(raw_line, &["User:", "用户:", "Human:"]) {
        return (HistoryRole::User, rest.trim());
    }
    if let Some(rest) = strip_role_prefix(raw_line, &["Assistant:", "助手:", "AI:"]) {
        return (HistoryRole::Assistant, rest.trim());
    }
    (HistoryRole::Unknown, raw_line.trim())
}

fn strip_role_prefix<'a>(line: &'a str, prefixes: &[&str]) -> Option<&'a str> {
    for prefix in prefixes {
        if let Some(rest) = strip_prefix_case_insensitive(line, prefix) {
            return Some(rest);
        }
    }
    None
}

fn strip_prefix_case_insensitive<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    if line.len() < prefix.len() {
        return None;
    }
    let head = line.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        return line.get(prefix.len()..);
    }
    None
}

fn is_enhance_status_line(line: &str) -> bool {
    let compact = line.trim();
    if compact.is_empty() {
        return false;
    }

    let lower = compact.to_ascii_lowercase();
    if lower.contains("enhancer")
        && (lower.contains("trigger")
            || lower.contains("provider")
            || lower.contains("retry")
            || lower.contains("re-run")
            || lower.contains("re run"))
    {
        return true;
    }

    if compact.contains("enhancer")
        && (compact.contains("已触发")
            || compact.contains("provider 参数")
            || compact.contains("参数不兼容")
            || compact.contains("重新执行")
            || compact.contains("继续创建项目")
            || compact.contains("继续搭建项目"))
    {
        return true;
    }

    false
}

fn build_enhance_cache_key(
    project_root_path: &PathBuf,
    provider_kind: EnhanceProviderKind,
    prompt: &str,
    history: &str,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    project_root_path.to_string_lossy().hash(&mut hasher);
    provider_kind.as_str().hash(&mut hasher);
    prompt.hash(&mut hasher);
    history.hash(&mut hasher);
    hasher.finish()
}

fn enhance_result_cache() -> &'static Mutex<HashMap<u64, (Instant, String)>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, (Instant, String)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lookup_recent_enhance_result(cache_key: u64, ttl: Duration) -> Option<String> {
    let mut guard = enhance_result_cache().lock().ok()?;
    let now = Instant::now();
    guard.retain(|_, (at, _)| now.duration_since(*at) <= ttl);
    guard.get(&cache_key).map(|(_, result)| result.clone())
}

fn store_recent_enhance_result(cache_key: u64, result: &str) {
    if let Ok(mut guard) = enhance_result_cache().lock() {
        guard.insert(cache_key, (Instant::now(), result.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Config, EnhanceProviderKind, derive_prompt_from_history, prepare_continue_inputs,
        resolve_effective_prompt, resolve_provider_kind, strip_enhance_markers,
    };
    use serde_json::json;
    use std::collections::HashSet;

    fn test_config(provider: &str) -> Config {
        Config {
            base_url: "https://example.com".to_string(),
            token: "token".to_string(),
            batch_size: 10,
            max_lines_per_blob: 800,
            text_extensions: HashSet::new(),
            exclude_patterns: Vec::new(),
            enable_log: false,
            search_provider: "remote".to_string(),
            enhance_provider: provider.to_string(),
            codex_api_base: "https://example.com/v1".to_string(),
            codex_api_key: "token".to_string(),
            codex_model: "gpt-5.4".to_string(),
            local_summary_mode: "gpt".to_string(),
            local_index_rebuild: "auto".to_string(),
            local_rerank_mode: "broad_only".to_string(),
            local_rerank_pool_size: 12,
            local_rerank_timeout_sec: 10,
            local_rerank_model: "gpt-5.4".to_string(),
            enhance_timeout_sec: 90,
            enhance_timeout_explicit: false,
            ui_timeout_sec: 480,
        }
    }

    #[test]
    fn strip_markers_basic() {
        let input = "Build login -enhancer please";
        assert_eq!(strip_enhance_markers(input), "Build login  please");
    }

    #[test]
    fn strip_markers_case_insensitive_with_punctuation() {
        let input = "Need API docs (-Enhance), then test.";
        assert_eq!(strip_enhance_markers(input), "Need API docs (), then test.");
    }

    #[test]
    fn preserve_newlines_and_spacing() {
        let input = "line1\n-enhancer\nline2  -enhance";
        assert_eq!(strip_enhance_markers(input), "line1\n\nline2  ");
    }

    #[test]
    fn strip_marker_attached_to_cjk_suffix() {
        let input = "做一个登录功能-enhancer";
        assert_eq!(strip_enhance_markers(input), "做一个登录功能");
    }

    #[test]
    fn keep_non_marker_word_fragment() {
        let input = "abc-enhancerd should stay";
        assert_eq!(strip_enhance_markers(input), "abc-enhancerd should stay");
    }

    #[test]
    fn derive_prompt_from_history_prefers_latest_user_line() {
        let history = "Assistant: 好的\nUser: 设计一个 Vue3 登录页 -enhancer";
        let prompt = derive_prompt_from_history(history).expect("history should produce prompt");
        assert_eq!(prompt, "设计一个 Vue3 登录页");
    }

    #[test]
    fn derive_prompt_from_history_ignores_assistant_status_noise() {
        let history = "User: 请实现一个登录页面 -enhancer\nAssistant: enhancer 已触发，但第一次因为 provider 参数不匹配失败了；我马上按当前环境 provider 重新执行一遍，然后继续创建项目。";
        let prompt = derive_prompt_from_history(history).expect("history should produce prompt");
        assert_eq!(prompt, "请实现一个登录页面");
    }

    #[test]
    fn derive_prompt_from_history_falls_back_to_unknown_before_assistant() {
        let history = "raw prompt line without role\nAssistant: 好的";
        let prompt = derive_prompt_from_history(history).expect("history should produce prompt");
        assert_eq!(prompt, "raw prompt line without role");
    }

    #[test]
    fn resolve_effective_prompt_falls_back_to_history_when_prompt_only_marker() {
        let prompt =
            resolve_effective_prompt("-enhancer", "用户: 用 Rust 写一个 CLI 工具 -enhancer")
                .expect("fallback prompt");
        assert_eq!(prompt, "用 Rust 写一个 CLI 工具");
    }

    #[test]
    fn resolve_effective_prompt_returns_none_when_no_signal() {
        let prompt = resolve_effective_prompt("-enhancer", "\n\n__END_CONVERSATION__\n");
        assert!(prompt.is_none());
    }

    #[test]
    fn codex_continue_drops_history() {
        let (prompt, history) = prepare_continue_inputs(
            EnhanceProviderKind::Codex,
            "current prompt",
            "long conversation history",
        );
        assert_eq!(prompt, "current prompt");
        assert!(history.is_empty());
    }

    #[test]
    fn codex_continue_keeps_full_prompt() {
        let input = "a".repeat(4500);
        let (prompt, history) = prepare_continue_inputs(EnhanceProviderKind::Codex, &input, "ctx");
        assert_eq!(prompt.chars().count(), 4500);
        assert!(history.is_empty());
    }

    #[test]
    fn remote_continue_keeps_history() {
        let (prompt, history) = prepare_continue_inputs(
            EnhanceProviderKind::Remote,
            "current prompt",
            "conversation history",
        );
        assert_eq!(prompt, "current prompt");
        assert_eq!(history, "conversation history");
    }

    #[test]
    fn resolve_provider_kind_uses_explicit_codex_when_configured_codex() {
        let args = json!({ "provider": "codex" });
        let cfg = test_config("codex");
        let kind = resolve_provider_kind(&args, &cfg).expect("provider should parse");
        assert_eq!(kind, EnhanceProviderKind::Codex);
    }

    #[test]
    fn resolve_provider_kind_rejects_invalid_override() {
        let args = json!({ "provider": "codx" });
        let cfg = test_config("remote");
        let err = resolve_provider_kind(&args, &cfg).unwrap_err();
        assert!(err.contains("Invalid provider override"));
    }

    #[test]
    fn resolve_provider_kind_ignores_mismatched_override() {
        let args = json!({ "provider": "remote" });
        let cfg = test_config("codex");
        let kind =
            resolve_provider_kind(&args, &cfg).expect("provider should fallback to configured");
        assert_eq!(kind, EnhanceProviderKind::Codex);
    }

    #[test]
    fn resolve_provider_kind_allows_same_override() {
        let args = json!({ "provider": "codex" });
        let cfg = test_config("codex");
        let kind = resolve_provider_kind(&args, &cfg).expect("provider should parse");
        assert_eq!(kind, EnhanceProviderKind::Codex);
    }
}

fn resolve_provider_kind(
    args: &serde_json::Value,
    config: &Config,
) -> Result<EnhanceProviderKind, String> {
    let configured = EnhanceProviderKind::parse(&config.enhance_provider).ok_or_else(|| {
        format!(
            "Invalid configured provider: {} (expected remote|codex)",
            config.enhance_provider
        )
    })?;

    if let Some(raw) = args.get("provider").and_then(|v| v.as_str()) {
        let override_kind = EnhanceProviderKind::parse(raw)
            .ok_or_else(|| format!("Invalid provider override: {} (expected remote|codex)", raw))?;
        if override_kind != configured {
            log_debug(format!(
                "enhance_prompt: provider override ignored configured={} requested={}",
                configured.as_str(),
                override_kind.as_str()
            ));
        }
        return Ok(configured);
    }

    Ok(configured)
}

fn resolve_search_provider_kind(config: &Config) -> Result<SearchProviderKind, String> {
    SearchProviderKind::parse(&config.search_provider).ok_or_else(|| {
        format!(
            "Invalid configured search provider: {} (expected remote|local)",
            config.search_provider
        )
    })
}

fn resolve_local_summary_mode(config: &Config) -> Result<LocalSummaryMode, String> {
    LocalSummaryMode::parse(&config.local_summary_mode).ok_or_else(|| {
        format!(
            "Invalid configured local summary mode: {} (expected gpt|local_fallback_only)",
            config.local_summary_mode
        )
    })
}

fn resolve_local_rerank_mode(config: &Config) -> Result<LocalRerankMode, String> {
    LocalRerankMode::parse(&config.local_rerank_mode).ok_or_else(|| {
        format!(
            "Invalid configured local rerank mode: {} (expected off|broad_only)",
            config.local_rerank_mode
        )
    })
}

fn resolve_local_index_rebuild_mode(config: &Config) -> Result<LocalIndexRebuildMode, String> {
    LocalIndexRebuildMode::parse(&config.local_index_rebuild).ok_or_else(|| {
        format!(
            "Invalid configured local index rebuild mode: {} (expected auto|force_full)",
            config.local_index_rebuild
        )
    })
}
fn resolve_enhance_timeout_sec(config: &Config, provider_kind: EnhanceProviderKind) -> u64 {
    if config.enhance_timeout_explicit {
        return config.enhance_timeout_sec;
    }

    match provider_kind {
        EnhanceProviderKind::Remote => config.enhance_timeout_sec,
        EnhanceProviderKind::Codex => config.enhance_timeout_sec.max(180),
    }
}

fn classify_enhance_error(err: &str) -> &'static str {
    let lower = err.to_ascii_lowercase();
    if lower.contains("timeout") {
        "timeout"
    } else if lower.contains("token") || lower.contains("401") {
        "auth"
    } else if lower.contains("403") {
        "forbidden"
    } else if lower.contains("429") || lower.contains("rate limit") {
        "rate_limit"
    } else if lower.contains("connect") || lower.contains("dns") || lower.contains("refused") {
        "network"
    } else {
        "unknown"
    }
}

/// 构造 MCP logging 通道的发送闭包。
fn build_mcp_sender(logger: McpLogger) -> logging::McpLogSender {
    Arc::new(move |level, message| {
        let level_str = match level {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warning => "warning",
            LogLevel::Error => "error",
        };
        logger.send(level_str, message);
    })
}
