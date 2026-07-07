use crate::enhancer::provider::EnhanceProviderKind;
use crate::index::{LocalIndexRebuildMode, LocalRerankMode, LocalSummaryMode, SearchProviderKind};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 运行时配置，来源于 CLI 参数、本地配置文件、环境变量与内置默认值。
///
/// 插件化后每次 CLI 只执行一个能力，因此远端必填项不在这里做全局强校验；
/// 具体能力会在真正需要 remote provider 时 fail-fast。
#[derive(Clone, Debug)]
pub struct Config {
    pub base_url: String,
    pub token: String,
    #[allow(dead_code)]
    pub batch_size: usize,
    pub max_lines_per_blob: usize,
    pub text_extensions: HashSet<String>,
    pub exclude_patterns: Vec<String>,
    pub enable_log: bool,
    pub search_provider: String,
    pub enhance_provider: String,
    pub codex_api_base: String,
    pub codex_api_key: String,
    pub codex_model: String,
    pub codex_reasoning_effort: String,
    pub debug: bool,
    pub debug_verbose: bool,
    pub debug_file: String,
    pub local_summary_mode: String,
    pub local_index_rebuild: String,
    pub local_rerank_mode: String,
    pub local_rerank_pool_size: usize,
    pub local_rerank_timeout_sec: u64,
    pub local_rerank_model: String,
    pub search_timeout_sec: u64,
    pub enhance_timeout_sec: u64,
    pub enhance_timeout_explicit: bool,
    pub ui_timeout_sec: u64,
}

/// CLI 参数的中间解析结果。
///
/// 这里不做强校验，统一在 `init_config` 中处理缺失项。
struct ParsedArgs {
    project_root_path: Option<String>,
    base_url: Option<String>,
    token: Option<String>,
    batch_size: Option<usize>,
    max_lines_per_blob: Option<usize>,
    enable_log: Option<bool>,
    search_provider: Option<String>,
    enhance_provider: Option<String>,
    codex_api_base: Option<String>,
    codex_api_key: Option<String>,
    codex_model: Option<String>,
    codex_reasoning_effort: Option<String>,
    debug: Option<bool>,
    debug_verbose: Option<bool>,
    debug_file: Option<String>,
    local_summary_mode: Option<String>,
    local_index_rebuild: Option<String>,
    local_rerank_mode: Option<String>,
    local_rerank_pool_size: Option<usize>,
    local_rerank_timeout_sec: Option<u64>,
    local_rerank_model: Option<String>,
    search_timeout_sec: Option<u64>,
    enhance_timeout_sec: Option<u64>,
    ui_timeout_sec: Option<u64>,
}

/// 本地配置文件结构。
///
/// 字段使用 camelCase 面向 JSON 用户，同时保留 snake_case alias，避免从旧文档或
/// 手写配置迁移时因为命名风格不同直接失效。
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct FileConfig {
    #[serde(alias = "base_url")]
    base_url: Option<String>,
    token: Option<String>,
    #[serde(alias = "batch_size")]
    batch_size: Option<usize>,
    #[serde(alias = "max_lines_per_blob")]
    max_lines_per_blob: Option<usize>,
    #[serde(alias = "text_extensions")]
    text_extensions: Option<Vec<String>>,
    #[serde(alias = "exclude_patterns")]
    exclude_patterns: Option<Vec<String>>,
    #[serde(alias = "enable_log")]
    enable_log: Option<bool>,
    #[serde(alias = "search_provider")]
    search_provider: Option<String>,
    #[serde(alias = "enhance_provider", alias = "provider")]
    enhance_provider: Option<String>,
    #[serde(alias = "codex_api_base")]
    codex_api_base: Option<String>,
    #[serde(alias = "codex_api_key")]
    codex_api_key: Option<String>,
    #[serde(alias = "codex_model")]
    codex_model: Option<String>,
    #[serde(alias = "codex_reasoning_effort")]
    codex_reasoning_effort: Option<String>,
    debug: Option<bool>,
    #[serde(alias = "debug_verbose")]
    debug_verbose: Option<bool>,
    #[serde(alias = "debug_file")]
    debug_file: Option<String>,
    #[serde(alias = "local_summary_mode")]
    local_summary_mode: Option<String>,
    #[serde(alias = "local_index_rebuild")]
    local_index_rebuild: Option<String>,
    #[serde(alias = "local_rerank_mode")]
    local_rerank_mode: Option<String>,
    #[serde(alias = "local_rerank_pool_size")]
    local_rerank_pool_size: Option<usize>,
    #[serde(alias = "local_rerank_timeout_sec")]
    local_rerank_timeout_sec: Option<u64>,
    #[serde(alias = "local_rerank_model")]
    local_rerank_model: Option<String>,
    #[serde(alias = "search_timeout_sec")]
    search_timeout_sec: Option<u64>,
    #[serde(alias = "enhance_timeout_sec")]
    enhance_timeout_sec: Option<u64>,
    #[serde(alias = "ui_timeout_sec")]
    ui_timeout_sec: Option<u64>,
}

impl FileConfig {
    fn merge_from(&mut self, higher_priority: FileConfig) {
        merge_option(&mut self.base_url, higher_priority.base_url);
        merge_option(&mut self.token, higher_priority.token);
        merge_option(&mut self.batch_size, higher_priority.batch_size);
        merge_option(
            &mut self.max_lines_per_blob,
            higher_priority.max_lines_per_blob,
        );
        merge_option(&mut self.text_extensions, higher_priority.text_extensions);
        merge_option(&mut self.exclude_patterns, higher_priority.exclude_patterns);
        merge_option(&mut self.enable_log, higher_priority.enable_log);
        merge_option(&mut self.search_provider, higher_priority.search_provider);
        merge_option(&mut self.enhance_provider, higher_priority.enhance_provider);
        merge_option(&mut self.codex_api_base, higher_priority.codex_api_base);
        merge_option(&mut self.codex_api_key, higher_priority.codex_api_key);
        merge_option(&mut self.codex_model, higher_priority.codex_model);
        merge_option(
            &mut self.codex_reasoning_effort,
            higher_priority.codex_reasoning_effort,
        );
        merge_option(&mut self.debug, higher_priority.debug);
        merge_option(&mut self.debug_verbose, higher_priority.debug_verbose);
        merge_option(&mut self.debug_file, higher_priority.debug_file);
        merge_option(
            &mut self.local_summary_mode,
            higher_priority.local_summary_mode,
        );
        merge_option(
            &mut self.local_index_rebuild,
            higher_priority.local_index_rebuild,
        );
        merge_option(
            &mut self.local_rerank_mode,
            higher_priority.local_rerank_mode,
        );
        merge_option(
            &mut self.local_rerank_pool_size,
            higher_priority.local_rerank_pool_size,
        );
        merge_option(
            &mut self.local_rerank_timeout_sec,
            higher_priority.local_rerank_timeout_sec,
        );
        merge_option(
            &mut self.local_rerank_model,
            higher_priority.local_rerank_model,
        );
        merge_option(
            &mut self.search_timeout_sec,
            higher_priority.search_timeout_sec,
        );
        merge_option(
            &mut self.enhance_timeout_sec,
            higher_priority.enhance_timeout_sec,
        );
        merge_option(&mut self.ui_timeout_sec, higher_priority.ui_timeout_sec);
    }
}

/// 解析命令行并构造最终配置。
///
/// # 返回
/// - `Ok(Config)`：参数完整且合法
/// - `Err(String)`：缺失参数或无法规范化
pub fn init_config() -> Result<Config, String> {
    let args = parse_args();
    let file_config = load_local_config(args.project_root_path.as_deref())?;

    let search_provider_name = resolve_string(
        args.search_provider,
        file_config.search_provider.as_deref(),
        "ACE_TOOL_SEARCH_PROVIDER",
        "local",
    );
    let search_provider_kind =
        SearchProviderKind::parse(&search_provider_name).ok_or_else(|| {
            format!(
                "Invalid search provider: {} (expected remote|local)",
                search_provider_name
            )
        })?;

    let base_url = resolve_optional_string(
        args.base_url,
        file_config.base_url.as_deref(),
        "ACE_TOOL_BASE_URL",
    );
    let token = resolve_optional_string(args.token, file_config.token.as_deref(), "ACE_TOOL_TOKEN");
    let batch_size = resolve_usize(
        args.batch_size,
        file_config.batch_size,
        "ACE_TOOL_BATCH_SIZE",
        10,
        1,
        100,
    );
    let max_lines_per_blob = resolve_usize(
        args.max_lines_per_blob,
        file_config.max_lines_per_blob,
        "ACE_TOOL_MAX_LINES_PER_BLOB",
        800,
        50,
        5000,
    );
    let text_extensions = resolve_text_extensions(file_config.text_extensions);
    let exclude_patterns = resolve_string_list(
        file_config.exclude_patterns,
        "ACE_TOOL_EXCLUDE_PATTERNS",
        default_exclude_patterns(),
    );

    let provider_name = resolve_string(
        args.enhance_provider.clone(),
        file_config.enhance_provider.as_deref(),
        "ACE_TOOL_ENHANCE_PROVIDER",
        "remote",
    );
    let provider_kind = EnhanceProviderKind::parse(&provider_name).ok_or_else(|| {
        format!(
            "Invalid provider: {} (expected remote|codex)",
            provider_name
        )
    })?;

    let mut base_url = base_url.unwrap_or_default();
    let token = token.unwrap_or_default();
    if !base_url.trim().is_empty() {
        base_url = normalize_base_url(&base_url);
    }
    if base_url.starts_with("http://") {
        let original = base_url.clone();
        // 保证走 https，避免被远端拒绝或降级为不安全连接。
        base_url = base_url.replacen("http://", "https://", 1);
        eprintln!(
            "Auto converted http:// to https:// ({} -> {})",
            original, base_url
        );
    }
    base_url = base_url.trim_end_matches('/').to_string();

    let codex_api_base = resolve_optional_string(
        args.codex_api_base,
        file_config.codex_api_base.as_deref(),
        "ACE_TOOL_CODEX_API_BASE",
    )
    .map(|value| normalize_external_base_url(&value));
    let codex_api_key = resolve_optional_string(
        args.codex_api_key,
        file_config.codex_api_key.as_deref(),
        "ACE_TOOL_CODEX_API_KEY",
    );
    let codex_model = resolve_string(
        args.codex_model,
        file_config.codex_model.as_deref(),
        "ACE_TOOL_CODEX_MODEL",
        "gpt-5.4",
    );
    let codex_reasoning_effort = resolve_string(
        args.codex_reasoning_effort,
        file_config.codex_reasoning_effort.as_deref(),
        "ACE_TOOL_CODEX_REASONING_EFFORT",
        "low",
    );
    validate_codex_reasoning_effort(&codex_reasoning_effort)?;
    let debug = resolve_bool(args.debug, file_config.debug, "ACE_TOOL_DEBUG", false);
    let debug_verbose = resolve_bool(
        args.debug_verbose,
        file_config.debug_verbose,
        "ACE_TOOL_DEBUG_VERBOSE",
        false,
    );
    let debug_file = resolve_string(
        args.debug_file,
        file_config.debug_file.as_deref(),
        "ACE_TOOL_DEBUG_FILE",
        "",
    );
    let local_summary_mode_name = resolve_string(
        args.local_summary_mode,
        file_config.local_summary_mode.as_deref(),
        "ACE_TOOL_LOCAL_SUMMARY_MODE",
        "local_fallback_only",
    );
    let local_summary_mode =
        LocalSummaryMode::parse(&local_summary_mode_name).ok_or_else(|| {
            format!(
                "Invalid local summary mode: {} (expected gpt|local_fallback_only)",
                local_summary_mode_name
            )
        })?;
    let local_index_rebuild_name = resolve_string(
        args.local_index_rebuild,
        file_config.local_index_rebuild.as_deref(),
        "ACE_TOOL_LOCAL_INDEX_REBUILD",
        "auto",
    );
    let local_index_rebuild =
        LocalIndexRebuildMode::parse(&local_index_rebuild_name).ok_or_else(|| {
            format!(
                "Invalid local index rebuild mode: {} (expected auto|force_full)",
                local_index_rebuild_name
            )
        })?;
    let local_rerank_mode_name = resolve_string(
        args.local_rerank_mode,
        file_config.local_rerank_mode.as_deref(),
        "ACE_TOOL_LOCAL_RERANK_MODE",
        "off",
    );
    let local_rerank_mode = LocalRerankMode::parse(&local_rerank_mode_name).ok_or_else(|| {
        format!(
            "Invalid local rerank mode: {} (expected off|broad_only)",
            local_rerank_mode_name
        )
    })?;
    let local_rerank_pool_size = resolve_usize(
        args.local_rerank_pool_size,
        file_config.local_rerank_pool_size,
        "ACE_TOOL_LOCAL_RERANK_POOL_SIZE",
        12,
        4,
        32,
    );
    let local_rerank_timeout_sec = resolve_u64(
        args.local_rerank_timeout_sec,
        file_config.local_rerank_timeout_sec,
        "ACE_TOOL_LOCAL_RERANK_TIMEOUT_SEC",
        10,
        3,
        120,
    );
    let local_rerank_model = resolve_string(
        args.local_rerank_model,
        file_config.local_rerank_model.as_deref(),
        "ACE_TOOL_LOCAL_RERANK_MODEL",
        &codex_model,
    );
    let search_timeout_sec = resolve_u64(
        args.search_timeout_sec,
        file_config.search_timeout_sec,
        "ACE_TOOL_SEARCH_TIMEOUT_SEC",
        50,
        10,
        300,
    );
    let enhance_timeout_override = resolve_u64_override(
        args.enhance_timeout_sec,
        file_config.enhance_timeout_sec,
        "ACE_TOOL_ENHANCE_TIMEOUT_SEC",
        10,
        600,
    );

    Ok(Config {
        base_url,
        token,
        batch_size,
        max_lines_per_blob,
        text_extensions,
        exclude_patterns,
        enable_log: resolve_bool(
            args.enable_log,
            file_config.enable_log,
            "ACE_TOOL_ENABLE_LOG",
            false,
        ),
        search_provider: search_provider_kind.as_str().to_string(),
        enhance_provider: provider_kind.as_str().to_string(),
        codex_api_base: codex_api_base.unwrap_or_default(),
        codex_api_key: codex_api_key.unwrap_or_default(),
        codex_model,
        codex_reasoning_effort,
        debug,
        debug_verbose,
        debug_file,
        local_summary_mode: local_summary_mode.as_str().to_string(),
        local_index_rebuild: local_index_rebuild.as_str().to_string(),
        local_rerank_mode: local_rerank_mode.as_str().to_string(),
        local_rerank_pool_size,
        local_rerank_timeout_sec,
        local_rerank_model,
        search_timeout_sec,
        enhance_timeout_sec: enhance_timeout_override.unwrap_or(90),
        enhance_timeout_explicit: enhance_timeout_override.is_some(),
        ui_timeout_sec: resolve_u64(
            args.ui_timeout_sec,
            file_config.ui_timeout_sec,
            "ACE_TOOL_UI_TIMEOUT_SEC",
            8 * 60,
            30,
            3600,
        ),
    })
}

/// 解析 CLI 参数，保持最小逻辑以降低解析出错风险。
fn parse_args() -> ParsedArgs {
    let mut result = ParsedArgs {
        project_root_path: None,
        base_url: None,
        token: None,
        batch_size: None,
        max_lines_per_blob: None,
        enable_log: None,
        search_provider: None,
        enhance_provider: None,
        codex_api_base: None,
        codex_api_key: None,
        codex_model: None,
        codex_reasoning_effort: None,
        debug: None,
        debug_verbose: None,
        debug_file: None,
        local_summary_mode: None,
        local_index_rebuild: None,
        local_rerank_mode: None,
        local_rerank_pool_size: None,
        local_rerank_timeout_sec: None,
        local_rerank_model: None,
        search_timeout_sec: None,
        enhance_timeout_sec: None,
        ui_timeout_sec: None,
    };

    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--project-root" | "--project-root-path" => {
                if let Some(value) = iter.next() {
                    result.project_root_path = Some(value);
                }
            }
            "--base-url" => {
                if let Some(value) = iter.next() {
                    result.base_url = Some(value);
                }
            }
            "--token" => {
                if let Some(value) = iter.next() {
                    result.token = Some(value);
                }
            }
            "--batch-size" => {
                if let Some(value) = iter.next() {
                    result.batch_size = value.trim().parse::<usize>().ok();
                }
            }
            "--max-lines-per-blob" => {
                if let Some(value) = iter.next() {
                    result.max_lines_per_blob = value.trim().parse::<usize>().ok();
                }
            }
            "--enable-log" => {
                result.enable_log = Some(true);
            }
            "--search-provider" => {
                if let Some(value) = iter.next() {
                    result.search_provider = Some(value);
                }
            }
            "--provider" => {
                if let Some(value) = iter.next() {
                    result.enhance_provider = Some(value);
                }
            }
            "--codex-api-base" => {
                if let Some(value) = iter.next() {
                    result.codex_api_base = Some(value);
                }
            }
            "--codex-api-key" => {
                if let Some(value) = iter.next() {
                    result.codex_api_key = Some(value);
                }
            }
            "--codex-model" => {
                if let Some(value) = iter.next() {
                    result.codex_model = Some(value);
                }
            }
            "--codex-reasoning-effort" => {
                if let Some(value) = iter.next() {
                    result.codex_reasoning_effort = Some(value);
                }
            }
            "--debug" => {
                result.debug = Some(true);
            }
            "--debug-verbose" => {
                result.debug_verbose = Some(true);
            }
            "--debug-file" => {
                if let Some(value) = iter.next() {
                    result.debug_file = Some(value);
                }
            }
            "--local-summary-mode" => {
                if let Some(value) = iter.next() {
                    result.local_summary_mode = Some(value);
                }
            }
            "--local-index-rebuild" => {
                if let Some(value) = iter.next() {
                    result.local_index_rebuild = Some(value);
                }
            }
            "--local-rerank-mode" => {
                if let Some(value) = iter.next() {
                    result.local_rerank_mode = Some(value);
                }
            }
            "--local-rerank-pool-size" => {
                if let Some(value) = iter.next() {
                    result.local_rerank_pool_size = value.trim().parse::<usize>().ok();
                }
            }
            "--local-rerank-timeout-sec" => {
                if let Some(value) = iter.next() {
                    result.local_rerank_timeout_sec = value.trim().parse::<u64>().ok();
                }
            }
            "--local-rerank-model" => {
                if let Some(value) = iter.next() {
                    result.local_rerank_model = Some(value);
                }
            }
            "--search-timeout-sec" => {
                if let Some(value) = iter.next() {
                    result.search_timeout_sec = value.trim().parse::<u64>().ok();
                }
            }
            "--enhance-timeout-sec" => {
                if let Some(value) = iter.next() {
                    result.enhance_timeout_sec = value.trim().parse::<u64>().ok();
                }
            }
            "--ui-timeout-sec" => {
                if let Some(value) = iter.next() {
                    result.ui_timeout_sec = value.trim().parse::<u64>().ok();
                }
            }
            _ => {}
        }
    }

    result
}

fn load_local_config(project_root_path: Option<&str>) -> Result<FileConfig, String> {
    let mut config = FileConfig::default();

    // 用户级配置提供跨项目默认值，适合 token、网关地址和模型等稳定个人配置。
    if let Some(user_path) = user_config_path() {
        config.merge_from(read_config_file(&user_path)?);
    }

    // 项目级配置覆盖用户级配置，适合每个仓库单独选择 provider、rerank 策略和超时。
    if let Some(project_root_path) = project_root_path {
        let project_path = PathBuf::from(project_root_path)
            .join(".ace-tool")
            .join("config.json");
        config.merge_from(read_config_file(&project_path)?);
    }

    Ok(config)
}

fn user_config_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .map(|home| home.join(".ace-tool").join("config.json"))
}

fn read_config_file(path: &Path) -> Result<FileConfig, String> {
    if !path.exists() {
        return Ok(FileConfig::default());
    }
    if !path.is_file() {
        return Err(format!(
            "ACE Tool config path is not a file: {}",
            path.display()
        ));
    }

    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("Failed to read ACE Tool config {}: {}", path.display(), err))?;
    if raw.trim().is_empty() {
        return Ok(FileConfig::default());
    }

    serde_json::from_str::<FileConfig>(&raw)
        .map_err(|err| format!("Invalid ACE Tool config {}: {}", path.display(), err))
}

fn merge_option<T>(current: &mut Option<T>, higher_priority: Option<T>) {
    if higher_priority.is_some() {
        *current = higher_priority;
    }
}

fn resolve_optional_string(
    cli_value: Option<String>,
    file_value: Option<&str>,
    env_key: &str,
) -> Option<String> {
    if let Some(value) = cli_value {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(value) = file_value {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    std::env::var(env_key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_string(
    cli_value: Option<String>,
    file_value: Option<&str>,
    env_key: &str,
    default: &str,
) -> String {
    if let Some(value) = cli_value {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Some(value) = file_value {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Ok(value) = std::env::var(env_key) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    default.to_string()
}

fn resolve_u64(
    cli_value: Option<u64>,
    file_value: Option<u64>,
    env_key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> u64 {
    resolve_u64_override(cli_value, file_value, env_key, min, max).unwrap_or(default)
}

fn resolve_u64_override(
    cli_value: Option<u64>,
    file_value: Option<u64>,
    env_key: &str,
    min: u64,
    max: u64,
) -> Option<u64> {
    let from_cli = cli_value.filter(|value| *value >= min && *value <= max);
    if let Some(value) = from_cli {
        return Some(value);
    }

    let from_file = file_value.filter(|value| *value >= min && *value <= max);
    if let Some(value) = from_file {
        return Some(value);
    }

    std::env::var(env_key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value >= min && *value <= max)
}

fn resolve_usize(
    cli_value: Option<usize>,
    file_value: Option<usize>,
    env_key: &str,
    default: usize,
    min: usize,
    max: usize,
) -> usize {
    let from_cli = cli_value.filter(|value| *value >= min && *value <= max);
    if let Some(value) = from_cli {
        return value;
    }

    let from_file = file_value.filter(|value| *value >= min && *value <= max);
    if let Some(value) = from_file {
        return value;
    }

    std::env::var(env_key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value >= min && *value <= max)
        .unwrap_or(default)
}

fn resolve_bool(
    cli_value: Option<bool>,
    file_value: Option<bool>,
    env_key: &str,
    default: bool,
) -> bool {
    if let Some(value) = cli_value {
        return value;
    }
    if let Some(value) = file_value {
        return value;
    }
    std::env::var(env_key)
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(default)
}

fn validate_codex_reasoning_effort(value: &str) -> Result<(), String> {
    let normalized = value.trim().to_ascii_lowercase();
    if matches!(normalized.as_str(), "low" | "medium" | "high") {
        return Ok(());
    }

    Err(format!(
        "Invalid codex reasoning effort: {} (expected low|medium|high)",
        value
    ))
}

fn resolve_text_extensions(file_value: Option<Vec<String>>) -> HashSet<String> {
    let defaults = default_text_extensions();
    let values = match file_value {
        Some(values) => values,
        None => {
            return std::env::var("ACE_TOOL_TEXT_EXTENSIONS")
                .ok()
                .map(|raw| parse_string_list(&raw))
                .filter(|items| !items.is_empty())
                .map(normalize_extensions)
                .unwrap_or(defaults);
        }
    };

    if values.is_empty() {
        return defaults;
    }

    normalize_extensions(values)
}

fn normalize_extensions(values: Vec<String>) -> HashSet<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value.starts_with('.') {
                value
            } else {
                format!(".{value}")
            }
        })
        .collect()
}

fn resolve_string_list(
    file_value: Option<Vec<String>>,
    env_key: &str,
    default: Vec<String>,
) -> Vec<String> {
    if let Some(values) = file_value {
        let normalized = normalize_string_list(values);
        if !normalized.is_empty() {
            return normalized;
        }
    }

    std::env::var(env_key)
        .ok()
        .map(|raw| normalize_string_list(parse_string_list(&raw)))
        .filter(|items| !items.is_empty())
        .unwrap_or(default)
}

fn parse_string_list(raw: &str) -> Vec<String> {
    raw.split([';', ','])
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn normalize_string_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalize_external_base_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return trimmed.to_string();
    }

    format!("https://{}", trimmed)
}

/// 规范化 base-url，补齐协议前缀。
fn normalize_base_url(value: &str) -> String {
    if value.starts_with("http://") || value.starts_with("https://") {
        value.to_string()
    } else {
        format!("https://{}", value)
    }
}

/// 默认可索引的文本文件后缀集合。
fn default_text_extensions() -> HashSet<String> {
    let list = [
        ".py",
        ".js",
        ".ts",
        ".jsx",
        ".tsx",
        ".mjs",
        ".cjs",
        ".java",
        ".go",
        ".rs",
        ".cpp",
        ".c",
        ".cc",
        ".h",
        ".hpp",
        ".hxx",
        ".cs",
        ".rb",
        ".php",
        ".swift",
        ".kt",
        ".kts",
        ".scala",
        ".clj",
        ".cljs",
        ".lua",
        ".dart",
        ".m",
        ".mm",
        ".pl",
        ".pm",
        ".r",
        ".R",
        ".jl",
        ".ex",
        ".exs",
        ".erl",
        ".hs",
        ".zig",
        ".v",
        ".nim",
        ".f90",
        ".f95",
        ".groovy",
        ".gradle",
        ".sol",
        ".move",
        ".md",
        ".mdx",
        ".txt",
        ".json",
        ".jsonc",
        ".json5",
        ".yaml",
        ".yml",
        ".toml",
        ".xml",
        ".ini",
        ".conf",
        ".cfg",
        ".properties",
        ".env.example",
        ".editorconfig",
        ".html",
        ".htm",
        ".css",
        ".scss",
        ".sass",
        ".less",
        ".styl",
        ".vue",
        ".svelte",
        ".astro",
        ".ejs",
        ".hbs",
        ".pug",
        ".jade",
        ".jinja",
        ".jinja2",
        ".erb",
        ".liquid",
        ".twig",
        ".mustache",
        ".njk",
        ".sql",
        ".sh",
        ".bash",
        ".zsh",
        ".fish",
        ".ps1",
        ".psm1",
        ".bat",
        ".cmd",
        ".makefile",
        ".mk",
        ".cmake",
        ".graphql",
        ".gql",
        ".proto",
        ".prisma",
        ".csv",
        ".tsv",
        ".rst",
        ".adoc",
        ".tex",
        ".org",
        ".dockerfile",
        ".containerfile",
        ".vim",
        ".el",
        ".rkt",
    ];
    list.iter().map(|s| s.to_string()).collect()
}

/// 默认排除的路径/文件模式列表。
fn default_exclude_patterns() -> Vec<String> {
    vec![
        ".venv",
        "venv",
        ".env",
        "env",
        "node_modules",
        "vendor",
        ".pnpm",
        ".yarn",
        "bower_components",
        ".git",
        ".svn",
        ".hg",
        ".gitmodules",
        "__pycache__",
        ".pytest_cache",
        ".mypy_cache",
        ".tox",
        ".eggs",
        "*.egg-info",
        ".ruff_cache",
        "dist",
        "build",
        "target",
        "out",
        "bin",
        "obj",
        ".next",
        ".nuxt",
        ".output",
        ".vercel",
        ".netlify",
        ".turbo",
        ".parcel-cache",
        ".cache",
        ".temp",
        ".tmp",
        "coverage",
        ".nyc_output",
        "htmlcov",
        ".idea",
        ".vscode",
        ".vs",
        "*.swp",
        "*.swo",
        ".DS_Store",
        "Thumbs.db",
        "desktop.ini",
        "*.pyc",
        "*.pyo",
        "*.pyd",
        "*.so",
        "*.dll",
        "*.dylib",
        "*.exe",
        "*.o",
        "*.obj",
        "*.class",
        "*.jar",
        "*.war",
        "*.min.js",
        "*.min.css",
        "*.bundle.js",
        "*.chunk.js",
        "*.map",
        "*.gz",
        "*.zip",
        "*.tar",
        "*.rar",
        "package-lock.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "Gemfile.lock",
        "poetry.lock",
        "Cargo.lock",
        "composer.lock",
        "*.log",
        "logs",
        "tmp",
        "temp",
        "*.png",
        "*.jpg",
        "*.jpeg",
        "*.gif",
        "*.ico",
        "*.svg",
        "*.mp3",
        "*.mp4",
        "*.wav",
        "*.avi",
        "*.mov",
        "*.pdf",
        "*.doc",
        "*.docx",
        "*.xls",
        "*.xlsx",
        "*.woff",
        "*.woff2",
        "*.ttf",
        "*.eot",
        "*.otf",
        "*.db",
        "*.sqlite",
        "*.sqlite3",
        ".ace-tool",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::{FileConfig, read_config_file};
    use std::fs;

    #[test]
    fn read_config_file_accepts_camel_case_fields() {
        let dir = std::env::temp_dir().join(format!("ace-tool-config-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp config dir");
        let path = dir.join("config.json");
        fs::write(
            &path,
            r#"{
              "baseUrl": "acemcp.heroman.wtf/relay/",
              "searchProvider": "local",
              "enhanceProvider": "codex",
              "codexApiBase": "http://gateway.local/v1",
              "codexApiKey": "key",
              "codexModel": "gpt-5.4",
              "searchTimeoutSec": 60,
              "enhanceTimeoutSec": 180,
              "uiTimeoutSec": 600,
              "enableLog": true
            }"#,
        )
        .expect("write config");

        let config = read_config_file(&path).expect("read config");

        assert_eq!(
            config.base_url.as_deref(),
            Some("acemcp.heroman.wtf/relay/")
        );
        assert_eq!(config.search_provider.as_deref(), Some("local"));
        assert_eq!(config.enhance_provider.as_deref(), Some("codex"));
        assert_eq!(
            config.codex_api_base.as_deref(),
            Some("http://gateway.local/v1")
        );
        assert_eq!(config.search_timeout_sec, Some(60));
        assert_eq!(config.enhance_timeout_sec, Some(180));
        assert_eq!(config.ui_timeout_sec, Some(600));
        assert_eq!(config.enable_log, Some(true));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_config_merge_keeps_higher_priority_values() {
        let mut user_config = FileConfig {
            search_provider: Some("remote".to_string()),
            enhance_provider: Some("remote".to_string()),
            codex_model: Some("gpt-5.4".to_string()),
            search_timeout_sec: Some(50),
            ..Default::default()
        };
        let project_config = FileConfig {
            search_provider: Some("local".to_string()),
            search_timeout_sec: Some(90),
            ..Default::default()
        };

        user_config.merge_from(project_config);

        assert_eq!(user_config.search_provider.as_deref(), Some("local"));
        assert_eq!(user_config.enhance_provider.as_deref(), Some("remote"));
        assert_eq!(user_config.codex_model.as_deref(), Some("gpt-5.4"));
        assert_eq!(user_config.search_timeout_sec, Some(90));
    }

    #[test]
    fn read_config_file_accepts_snake_case_aliases() {
        let dir =
            std::env::temp_dir().join(format!("ace-tool-config-alias-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp config dir");
        let path = dir.join("config.json");
        fs::write(
            &path,
            r#"{
              "base_url": "https://example.com",
              "search_provider": "remote",
              "local_rerank_pool_size": 16,
              "search_timeout_sec": 70
            }"#,
        )
        .expect("write config");

        let config = read_config_file(&path).expect("read config");

        assert_eq!(config.base_url.as_deref(), Some("https://example.com"));
        assert_eq!(config.search_provider.as_deref(), Some("remote"));
        assert_eq!(config.local_rerank_pool_size, Some(16));
        assert_eq!(config.search_timeout_sec, Some(70));

        let _ = fs::remove_dir_all(&dir);
    }
}
