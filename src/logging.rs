use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 日志级别，用于文件日志与可选外部日志接收器。
#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
}

impl LogLevel {
    /// 获取可读的级别字符串。
    #[allow(dead_code)]
    fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO",
            LogLevel::Warning => "WARNING",
            LogLevel::Error => "ERROR",
        }
    }
}

/// 外部日志通道发送器。
pub type LogSender = Arc<dyn Fn(LogLevel, &str) + Send + Sync>;

/// 全局日志状态（外部 sender + 文件句柄）。
struct LoggerState {
    sender: Option<LogSender>,
    file: Option<File>,
    log_file_path: Option<PathBuf>,
}

static LOGGER: Mutex<LoggerState> = Mutex::new(LoggerState {
    sender: None,
    file: None,
    log_file_path: None,
});

/// 插件化后调试开关来自本地配置文件，环境变量只作为兼容旧入口的兜底。
#[derive(Clone, Debug)]
struct DebugConfig {
    enabled: bool,
    verbose: bool,
    file_path: Option<PathBuf>,
}

static DEBUG_CONFIG: OnceLock<DebugConfig> = OnceLock::new();

/// 初始化外部日志发送器。
#[allow(dead_code)]
pub fn init_log_sender(sender: LogSender) {
    let mut state = LOGGER.lock().unwrap();
    state.sender = Some(sender);
}

/// 在项目目录下启用文件日志（.ace-tool/ace-tool.log）。
pub fn enable_file_log(project_root: &Path) -> io::Result<()> {
    let ace_dir = project_root.join(".ace-tool");
    fs::create_dir_all(&ace_dir)?;
    let log_file_path = ace_dir.join("ace-tool.log");

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)?;

    let mut state = LOGGER.lock().unwrap();
    state.file = Some(file);
    state.log_file_path = Some(log_file_path);

    if let Some(file) = state.file.as_mut() {
        let timestamp = format_timestamp(SystemTime::now());
        let separator = format!(
            "\n{}\n{} | Session started\n{}\n",
            "=".repeat(60),
            timestamp,
            "=".repeat(60)
        );
        let _ = file.write_all(separator.as_bytes());
    }

    Ok(())
}

#[allow(dead_code)]
/// 发送日志到外部通道与本地文件。
pub fn send_log(level: LogLevel, message: &str) {
    let mut state = LOGGER.lock().unwrap();

    if let Some(sender) = state.sender.as_ref() {
        sender(level, message);
    }

    if let Some(file) = state.file.as_mut() {
        let timestamp = format_timestamp(SystemTime::now());
        let level_str = level.as_str();
        let log_line = format!("{} | {:<7} | {}\n", timestamp, level_str, message);
        let _ = file.write_all(log_line.as_bytes());
    }
}

/// 配置插件运行期调试日志。
///
/// 本地配置文件已经替代 MCP 参数成为主要配置入口；这里在进程启动后注入解析结果，
/// 避免日志模块继续直接绑定旧的环境变量事实源。OnceLock 保证配置只初始化一次，
/// 防止运行中反复改日志目标导致文件句柄和输出行为不可预测。
pub fn configure_debug_logging(enabled: bool, verbose: bool, file_path: Option<PathBuf>) {
    let _ = DEBUG_CONFIG.set(DebugConfig {
        enabled,
        verbose,
        file_path,
    });
}

/// 输出调试日志（stderr + 可选文件）。
///
/// 这些函数原先挂在 MCP 模块下，但插件化后 MCP 不再是主入口。
/// 日志属于横切基础设施，放在 logging 模块能避免业务模块继续依赖旧协议层。
pub fn log_debug(message: String) {
    if is_debug_enabled() {
        eprintln!("{message}");
        log_debug_to_file(&message);
    }
}

/// 输出更详细的调试日志（需开启 verbose）。
pub fn log_debug_verbose(message: String) {
    if is_debug_enabled() && is_debug_verbose_enabled() {
        eprintln!("{message}");
        log_debug_to_file(&message);
    }
}

/// 判断是否启用调试日志。
fn is_debug_enabled() -> bool {
    if let Some(config) = DEBUG_CONFIG.get() {
        return config.enabled;
    }

    static DEBUG: OnceLock<bool> = OnceLock::new();
    *DEBUG.get_or_init(|| {
        std::env::var("ACE_TOOL_DEBUG")
            .map(|value| {
                let normalized = value.trim().to_ascii_lowercase();
                matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
            })
            .unwrap_or(false)
    })
}

/// 判断是否启用详细调试日志。
fn is_debug_verbose_enabled() -> bool {
    if let Some(config) = DEBUG_CONFIG.get() {
        return config.verbose;
    }

    static VERBOSE: OnceLock<bool> = OnceLock::new();
    *VERBOSE.get_or_init(|| {
        std::env::var("ACE_TOOL_DEBUG_VERBOSE")
            .map(|value| {
                let normalized = value.trim().to_ascii_lowercase();
                matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
            })
            .unwrap_or(false)
    })
}

/// 将调试日志追加到文件（若已开启）。
fn log_debug_to_file(message: &str) {
    if let Some(writer) = debug_log_writer() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut guard = match writer.lock() {
            Ok(lock) => lock,
            Err(_) => return,
        };
        let _ = writeln!(guard, "[{timestamp}] {message}");
        let _ = guard.flush();
    }
}

/// 获取日志文件写入器（按需初始化）。
fn debug_log_writer() -> Option<&'static Mutex<File>> {
    static LOG_FILE: OnceLock<Option<Mutex<File>>> = OnceLock::new();
    LOG_FILE
        .get_or_init(|| {
            if !is_debug_enabled() {
                return None;
            }

            let path = debug_log_file_path();

            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }

            let file = OpenOptions::new().create(true).append(true).open(path);
            match file {
                Ok(handle) => Some(Mutex::new(handle)),
                Err(_) => None,
            }
        })
        .as_ref()
}

/// 解析调试日志文件路径：本地配置优先，旧环境变量其次，最后落到临时目录。
fn debug_log_file_path() -> PathBuf {
    if let Some(config) = DEBUG_CONFIG.get() {
        if let Some(path) = config.file_path.as_ref() {
            return path.clone();
        }
    }

    match std::env::var("ACE_TOOL_DEBUG_FILE") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
        _ => std::env::temp_dir().join("ace-tool.log"),
    }
}

#[allow(dead_code)]
/// 关闭文件日志句柄。
pub fn close_log() {
    let mut state = LOGGER.lock().unwrap();
    state.file = None;
}

/// 格式化时间戳。
fn format_timestamp(time: SystemTime) -> String {
    let datetime: chrono::DateTime<chrono::Local> = time.into();
    datetime.format("%Y-%m-%d %H:%M:%S").to_string()
}
