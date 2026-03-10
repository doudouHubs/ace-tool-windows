use crate::enhancer::provider::EnhanceProviderKind;
use std::collections::HashSet;

/// 运行时配置，来源于 CLI 参数与内置默认值。
///
/// 这些字段会在 MCP 工具调用时复用，避免重复解析参数。
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
    pub enhance_provider: String,
    pub codex_cmd: String,
    pub codex_reasoning_effort: String,
    pub enhance_timeout_sec: u64,
    pub enhance_timeout_explicit: bool,
    pub ui_timeout_sec: u64,
}

/// CLI 参数的中间解析结果。
///
/// 这里不做强校验，统一在 `init_config` 中处理缺失项。
struct ParsedArgs {
    base_url: Option<String>,
    token: Option<String>,
    enable_log: bool,
    enhance_provider: Option<String>,
    codex_cmd: Option<String>,
    codex_reasoning_effort: Option<String>,
    enhance_timeout_sec: Option<u64>,
    ui_timeout_sec: Option<u64>,
}

/// 解析命令行并构造最终配置。
///
/// # 返回
/// - `Ok(Config)`：参数完整且合法
/// - `Err(String)`：缺失参数或无法规范化
pub fn init_config() -> Result<Config, String> {
    let args = parse_args();
    let base_url = args
        .base_url
        .ok_or_else(|| "Missing required argument: --base-url".to_string())?;
    let token = args
        .token
        .ok_or_else(|| "Missing required argument: --token".to_string())?;

    let mut base_url = normalize_base_url(&base_url);
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

    let provider_name =
        resolve_string(args.enhance_provider, "ACE_TOOL_ENHANCE_PROVIDER", "remote");
    let provider_kind = EnhanceProviderKind::parse(&provider_name).ok_or_else(|| {
        format!(
            "Invalid provider: {} (expected remote|codex)",
            provider_name
        )
    })?;
    let codex_cmd = resolve_string(args.codex_cmd, "ACE_TOOL_CODEX_CMD", "codex");
    let codex_reasoning_effort = resolve_string(
        args.codex_reasoning_effort,
        "ACE_TOOL_CODEX_REASONING_EFFORT",
        "low",
    );
    let enhance_timeout_override = resolve_u64_override(
        args.enhance_timeout_sec,
        "ACE_TOOL_ENHANCE_TIMEOUT_SEC",
        10,
        600,
    );

    Ok(Config {
        base_url,
        token,
        batch_size: 10,
        max_lines_per_blob: 800,
        text_extensions: default_text_extensions(),
        exclude_patterns: default_exclude_patterns(),
        enable_log: args.enable_log,
        enhance_provider: provider_kind.as_str().to_string(),
        codex_cmd,
        codex_reasoning_effort,
        enhance_timeout_sec: enhance_timeout_override.unwrap_or(90),
        enhance_timeout_explicit: enhance_timeout_override.is_some(),
        ui_timeout_sec: resolve_u64(
            args.ui_timeout_sec,
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
        base_url: None,
        token: None,
        enable_log: false,
        enhance_provider: None,
        codex_cmd: None,
        codex_reasoning_effort: None,
        enhance_timeout_sec: None,
        ui_timeout_sec: None,
    };

    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
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
            "--enable-log" => {
                result.enable_log = true;
            }
            "--provider" => {
                if let Some(value) = iter.next() {
                    result.enhance_provider = Some(value);
                }
            }
            "--codex-cmd" => {
                if let Some(value) = iter.next() {
                    result.codex_cmd = Some(value);
                }
            }
            "--codex-reasoning-effort" => {
                if let Some(value) = iter.next() {
                    result.codex_reasoning_effort = Some(value);
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

fn resolve_string(cli_value: Option<String>, env_key: &str, default: &str) -> String {
    if let Some(value) = cli_value {
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

fn resolve_u64(cli_value: Option<u64>, env_key: &str, default: u64, min: u64, max: u64) -> u64 {
    resolve_u64_override(cli_value, env_key, min, max).unwrap_or(default)
}

fn resolve_u64_override(cli_value: Option<u64>, env_key: &str, min: u64, max: u64) -> Option<u64> {
    let from_cli = cli_value.filter(|value| *value >= min && *value <= max);
    if let Some(value) = from_cli {
        return Some(value);
    }

    std::env::var(env_key)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value >= min && *value <= max)
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
