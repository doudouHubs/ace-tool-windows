use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod schemas;

pub type ToolHandler = Arc<dyn Fn(&str, Option<Value>) -> Result<String, String> + Send + Sync>;

pub struct McpServer {
  tools: Vec<Value>,
  handler: ToolHandler,
  writer: Arc<Mutex<Box<dyn Write + Send>>>,
  frame_mode: Arc<Mutex<FrameMode>>,
}

#[derive(Clone)]
pub struct McpLogger {
  writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl McpServer {
  pub fn new(tools: Vec<Value>, handler: ToolHandler) -> Self {
    log_debug("mcp: server init".to_string());
    let writer: Box<dyn Write + Send> = Box::new(io::stdout());
    Self {
      tools,
      handler,
      writer: Arc::new(Mutex::new(writer)),
      frame_mode: Arc::new(Mutex::new(FrameMode::ContentLength)),
    }
  }

  pub fn logger(&self) -> McpLogger {
    McpLogger {
      writer: self.writer.clone(),
    }
  }

  pub fn run(&self) -> io::Result<()> {
    let stdin = io::stdin();
    let mut reader = io::BufReader::new(stdin.lock());

    loop {
      let (message, mode) = match read_message(&mut reader) {
        Ok(Some((msg, mode))) => (msg, mode),
        Ok(None) => {
          log_debug("mcp: stdin closed".to_string());
          break;
        }
        Err(err) => {
          log_debug(format!("mcp: read error: {err}"));
          continue;
        }
      };

      if message.is_null() {
        log_debug("mcp: received null message".to_string());
        continue;
      }

      self.update_frame_mode(mode);

      if let Some(method) = message.get("method").and_then(|v| v.as_str()) {
        let has_id = message.get("id").is_some();
        log_debug(format!("mcp: parsed message method={method} has_id={has_id}"));
      }

      let response = self.handle_message(&message);
      if let Some(resp) = response {
        if let Err(err) = self.write_message(&resp) {
          log_debug(format!("mcp: write error: {err}"));
        }
      }
    }

    Ok(())
  }

  #[allow(dead_code)]
  pub fn send_log(&self, level: &str, data: &str) {
    let logger = self.logger();
    logger.send(level, data);
  }

  fn handle_message(&self, message: &Value) -> Option<Value> {
    let method = message.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = match message.get("id") {
      Some(value) => value.clone(),
      None => {
        log_debug("mcp: message missing id".to_string());
        return None;
      }
    };
    log_debug(format!("mcp: handle message method={method}"));

    let params = message.get("params").cloned();

    match classify_method(method) {
      MethodKind::Initialize => {
        let protocol_version = select_protocol_version(params.as_ref());
        Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
          "protocolVersion": protocol_version,
          "capabilities": {
            "tools": {},
            "logging": {}
          },
          "serverInfo": {
            "name": "ace-tool",
            "version": "0.1.0"
          }
        }
      }))
      }
      MethodKind::ListTools => Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
          "tools": self.tools
        }
      })),
      MethodKind::CallTool => {
        let (tool_name, args) = extract_tool_call_params(params.as_ref());
        if tool_name.is_empty() {
          return Some(error_response(id, -32602, "Missing tool name"));
        }

        match (self.handler)(tool_name.as_str(), args) {
          Ok(text) => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
              "content": [
                {
                  "type": "text",
                  "text": text
                }
              ]
            }
          })),
          Err(err) => Some(error_response(id, -32000, &err)),
        }
      }
      MethodKind::Notification => None,
      MethodKind::Unknown => Some(error_response(id, -32601, "Method not found")),
    }
  }

  fn write_message(&self, message: &Value) -> io::Result<()> {
    let mut writer = self.writer.lock().unwrap();
    let data = serde_json::to_vec(message).map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    if let Some(id) = message.get("id") {
      log_debug(format!("mcp: send response id={id} bytes={}", data.len()));
    } else {
      log_debug(format!("mcp: send response bytes={}", data.len()));
    }
    let mode = *self.frame_mode.lock().unwrap();
    match mode {
      FrameMode::JsonLine => {
        writer.write_all(&data)?;
        writer.write_all(b"\n")?;
      }
      FrameMode::ContentLength => {
        write!(writer, "Content-Length: {}\r\n\r\n", data.len())?;
        writer.write_all(&data)?;
      }
      FrameMode::Unknown => {
        write!(writer, "Content-Length: {}\r\n\r\n", data.len())?;
        writer.write_all(&data)?;
      }
    }
    writer.flush()?;
    Ok(())
  }

  fn update_frame_mode(&self, mode: FrameMode) {
    if mode == FrameMode::Unknown {
      return;
    }
    let mut guard = self.frame_mode.lock().unwrap();
    if *guard != mode {
      log_debug(format!("mcp: switch frame mode to {mode:?}"));
      *guard = mode;
    }
  }
}

impl McpLogger {
  pub fn send(&self, level: &str, data: &str) {
    let payload = json!({
      "jsonrpc": "2.0",
      "method": "logging/message",
      "params": {
        "level": level,
        "data": data
      }
    });
    let mut writer = match self.writer.lock() {
      Ok(lock) => lock,
      Err(_) => return,
    };
    if let Ok(data) = serde_json::to_vec(&payload) {
      let _ = write!(writer, "Content-Length: {}\r\n\r\n", data.len());
      let _ = writer.write_all(&data);
      let _ = writer.flush();
    }
  }
}

#[derive(Debug, Clone, Copy)]
enum MethodKind {
  Initialize,
  ListTools,
  CallTool,
  Notification,
  Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameMode {
  ContentLength,
  JsonLine,
  Unknown,
}

fn classify_method(method: &str) -> MethodKind {
  let normalized = method.to_ascii_lowercase();
  match normalized.as_str() {
    "initialize" => MethodKind::Initialize,
    "notifications/initialized" | "initialized" => MethodKind::Notification,
    "tools/list" | "tools.list" | "list_tools" | "listtools" => MethodKind::ListTools,
    "tools/call" | "tools.call" | "call_tool" | "calltool" => MethodKind::CallTool,
    _ => MethodKind::Unknown,
  }
}

const LATEST_PROTOCOL_VERSION: &str = "DRAFT-2026-v1";
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 5] = [
  LATEST_PROTOCOL_VERSION,
  "2025-06-18",
  "2025-03-26",
  "2024-11-05",
  "2024-10-07",
];

fn select_protocol_version(params: Option<&Value>) -> String {
  let requested = params
    .and_then(|value| value.get("protocolVersion"))
    .and_then(|value| value.as_str());
  if let Some(version) = requested {
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&version) {
      return version.to_string();
    }
  }
  DEFAULT_PROTOCOL_VERSION.to_string()
}

pub(crate) fn log_debug(message: String) {
  if is_debug_enabled() {
    eprintln!("{message}");
    log_debug_to_file(&message);
  }
}

fn is_debug_enabled() -> bool {
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

pub(crate) fn log_debug_verbose(message: String) {
  if is_debug_enabled() && is_debug_verbose_enabled() {
    eprintln!("{message}");
    log_debug_to_file(&message);
  }
}

fn is_debug_verbose_enabled() -> bool {
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

fn sanitize_preview(input: &str, max_len: usize) -> String {
  let mut out = String::new();
  for ch in input.chars().take(max_len) {
    if ch == '\r' {
      out.push_str("\\r");
      continue;
    }
    if ch == '\n' {
      out.push_str("\\n");
      continue;
    }
    if ch.is_ascii_graphic() || ch == ' ' {
      out.push(ch);
    } else {
      out.push('.');
    }
  }
  if input.chars().count() > max_len {
    out.push_str("...");
  }
  out
}

fn debug_log_writer() -> Option<&'static Mutex<std::fs::File>> {
  static LOG_FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
  LOG_FILE.get_or_init(|| {
    if !is_debug_enabled() {
      return None;
    }

    let path = match std::env::var("ACE_TOOL_DEBUG_FILE") {
      Ok(value) if !value.trim().is_empty() => PathBuf::from(value),
      _ => std::env::temp_dir().join("ace-tool-mcp.log"),
    };

    let file = OpenOptions::new().create(true).append(true).open(path);
    match file {
      Ok(handle) => Some(Mutex::new(handle)),
      Err(_) => None,
    }
  })
  .as_ref()
}

fn extract_tool_call_params(params: Option<&Value>) -> (String, Option<Value>) {
  let params = match params {
    Some(value) => value,
    None => return (String::new(), None),
  };

  let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
  let args = params.get("arguments").cloned();
  (name, args)
}

fn error_response(id: Value, code: i32, message: &str) -> Value {
  json!({
    "jsonrpc": "2.0",
    "id": id,
    "error": {
      "code": code,
      "message": message
    }
  })
}

fn read_message(reader: &mut dyn BufRead) -> io::Result<Option<(Value, FrameMode)>> {
  let mut first_line = String::new();
  let bytes_read = reader.read_line(&mut first_line)?;
  if bytes_read == 0 {
    return Ok(None);
  }

  if first_line.trim().is_empty() {
    log_debug("mcp: read empty line".to_string());
    return Ok(Some((Value::Null, FrameMode::Unknown)));
  }

  let has_content_length = first_line.to_ascii_lowercase().starts_with("content-length:");
  log_debug(format!(
    "mcp: read header bytes={bytes_read} has_content_length={has_content_length}"
  ));
  log_debug_verbose(format!(
    "mcp: header preview={}",
    sanitize_preview(&first_line, 200)
  ));

  if has_content_length {
    let mut content_length = parse_content_length(&first_line).unwrap_or(0);
    let mut header_lines = 1usize;

    loop {
      let mut line = String::new();
      let read = reader.read_line(&mut line)?;
      if read == 0 {
        break;
      }
      let trimmed = line.trim();
      if trimmed.is_empty() {
        break;
      }
      header_lines += 1;
      if line.to_ascii_lowercase().starts_with("content-length:") {
        content_length = parse_content_length(&line).unwrap_or(content_length);
      }
    }

    log_debug(format!(
      "mcp: header parsed content_length={content_length} header_lines={header_lines}"
    ));

    if content_length == 0 {
      log_debug("mcp: content length is zero".to_string());
      return Ok(Some((Value::Null, FrameMode::ContentLength)));
    }

    let mut buffer = vec![0u8; content_length];
    reader.read_exact(&mut buffer)?;
    log_debug(format!("mcp: read body bytes={}", buffer.len()));
    if let Ok(body_text) = std::str::from_utf8(&buffer) {
      log_debug_verbose(format!(
        "mcp: body preview={}",
        sanitize_preview(body_text, 200)
      ));
    }
    let value: Value = serde_json::from_slice(&buffer).map_err(|err| {
      log_debug(format!("mcp: json parse error: {err}"));
      io::Error::new(io::ErrorKind::InvalidData, err)
    })?;
    Ok(Some((value, FrameMode::ContentLength)))
  } else {
    let trimmed = first_line.trim_end();
    if trimmed.is_empty() {
      return Ok(Some((Value::Null, FrameMode::JsonLine)));
    }
    log_debug_verbose(format!(
      "mcp: line preview={}",
      sanitize_preview(trimmed, 200)
    ));
    let value: Value = serde_json::from_str(trimmed).map_err(|err| {
      log_debug(format!("mcp: json parse error: {err}"));
      io::Error::new(io::ErrorKind::InvalidData, err)
    })?;
    Ok(Some((value, FrameMode::JsonLine)))
  }
}

fn parse_content_length(line: &str) -> Option<usize> {
  let parts: Vec<&str> = line.split(':').collect();
  if parts.len() < 2 {
    return None;
  }
  parts[1].trim().parse::<usize>().ok()
}
