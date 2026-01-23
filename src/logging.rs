use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

#[allow(dead_code)]
#[derive(Clone, Copy)]
pub enum LogLevel {
  Debug,
  Info,
  Warning,
  Error,
}

impl LogLevel {
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

pub type McpLogSender = Arc<dyn Fn(LogLevel, &str) + Send + Sync>;

struct LoggerState {
  mcp_sender: Option<McpLogSender>,
  file: Option<File>,
  log_file_path: Option<PathBuf>,
}

static LOGGER: Mutex<LoggerState> = Mutex::new(LoggerState {
  mcp_sender: None,
  file: None,
  log_file_path: None,
});

pub fn init_mcp_logger(sender: McpLogSender) {
  let mut state = LOGGER.lock().unwrap();
  state.mcp_sender = Some(sender);
}

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
pub fn send_log(level: LogLevel, message: &str) {
  let mut state = LOGGER.lock().unwrap();

  if let Some(sender) = state.mcp_sender.as_ref() {
    sender(level, message);
  }

  if let Some(file) = state.file.as_mut() {
    let timestamp = format_timestamp(SystemTime::now());
    let level_str = level.as_str();
    let log_line = format!("{} | {:<7} | {}\n", timestamp, level_str, message);
    let _ = file.write_all(log_line.as_bytes());
  }
}

#[allow(dead_code)]
pub fn close_log() {
  let mut state = LOGGER.lock().unwrap();
  state.file = None;
}

fn format_timestamp(time: SystemTime) -> String {
  let datetime: chrono::DateTime<chrono::Local> = time.into();
  datetime.format("%Y-%m-%d %H:%M:%S").to_string()
}
