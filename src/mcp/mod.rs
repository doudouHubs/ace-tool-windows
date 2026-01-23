use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};

pub mod schemas;

pub type ToolHandler = Arc<dyn Fn(&str, Option<Value>) -> Result<String, String> + Send + Sync>;

pub struct McpServer {
  tools: Vec<Value>,
  handler: ToolHandler,
  writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

#[derive(Clone)]
pub struct McpLogger {
  writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl McpServer {
  pub fn new(tools: Vec<Value>, handler: ToolHandler) -> Self {
    let writer: Box<dyn Write + Send> = Box::new(io::stdout());
    Self {
      tools,
      handler,
      writer: Arc::new(Mutex::new(writer)),
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
      let message = match read_message(&mut reader) {
        Ok(Some(msg)) => msg,
        Ok(None) => break,
        Err(err) => {
          eprintln!("mcp: read error: {err}");
          continue;
        }
      };

      if message.is_null() {
        continue;
      }

      let response = self.handle_message(&message);
      if let Some(resp) = response {
        if let Err(err) = self.write_message(&resp) {
          eprintln!("mcp: write error: {err}");
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
      None => return None,
    };

    let params = message.get("params").cloned();

    match classify_method(method) {
      MethodKind::Initialize => Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
          "capabilities": {
            "tools": {},
            "logging": {}
          },
          "serverInfo": {
            "name": "ace-tool",
            "version": "0.1.0"
          }
        }
      })),
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
    write!(writer, "Content-Length: {}\r\n\r\n", data.len())?;
    writer.write_all(&data)?;
    writer.flush()?;
    Ok(())
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

fn read_message(reader: &mut dyn BufRead) -> io::Result<Option<Value>> {
  let mut first_line = String::new();
  let bytes_read = reader.read_line(&mut first_line)?;
  if bytes_read == 0 {
    return Ok(None);
  }

  if first_line.trim().is_empty() {
    return Ok(Some(Value::Null));
  }

  if first_line.to_ascii_lowercase().starts_with("content-length:") {
    let mut content_length = parse_content_length(&first_line).unwrap_or(0);

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
      if line.to_ascii_lowercase().starts_with("content-length:") {
        content_length = parse_content_length(&line).unwrap_or(content_length);
      }
    }

    if content_length == 0 {
      return Ok(Some(Value::Null));
    }

    let mut buffer = vec![0u8; content_length];
    reader.read_exact(&mut buffer)?;
    let value: Value = serde_json::from_slice(&buffer)
      .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    Ok(Some(value))
  } else {
    let trimmed = first_line.trim_end();
    if trimmed.is_empty() {
      return Ok(Some(Value::Null));
    }
    let value: Value = serde_json::from_str(trimmed)
      .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    Ok(Some(value))
  }
}

fn parse_content_length(line: &str) -> Option<usize> {
  let parts: Vec<&str> = line.split(':').collect();
  if parts.len() < 2 {
    return None;
  }
  parts[1].trim().parse::<usize>().ok()
}
