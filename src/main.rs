mod config;
mod enhancer;
mod index;
mod logging;
mod mcp;
mod ui;
mod utils;

use config::Config;
use enhancer::enhancer::PromptEnhancer;
use index::manager::IndexManager;
use logging::{init_mcp_logger, LogLevel};
use mcp::{log_debug, schemas, McpLogger, McpServer, ToolHandler};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::time::timeout;
use ui::session::{run_prompt_session, ContinueCallback, SessionAction, is_headless_mode};

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
    return Ok(format!("Error: Project path does not exist: {}", project_root_path));
  }
  if !project_root.is_dir() {
    return Ok(format!("Error: Project path is not a directory: {}", project_root_path));
  }

  if config.enable_log {
    let _ = logging::enable_file_log(&project_root);
  }

  let manager = IndexManager::new(
    project_root,
    config.base_url.clone(),
    config.token.clone(),
    config.text_extensions.clone(),
    config.max_lines_per_blob,
    config.exclude_patterns.clone(),
  )
  .map_err(|e| format!("Error: {e}"))?;

  let timeout_sec = std::env::var("ACE_TOOL_SEARCH_TIMEOUT_SEC")
    .ok()
    .and_then(|v| v.trim().parse::<u64>().ok())
    .filter(|v| *v >= 10 && *v <= 300)
    .unwrap_or(50);
  log_debug(format!(
    "search_context: start root={} query_len={} timeout={}s",
    project_root_path,
    query.chars().count(),
    timeout_sec
  ));

  let result = runtime.block_on(async {
    timeout(Duration::from_secs(timeout_sec), manager.search_context(query)).await
  });
  match result {
    Ok(text) => {
      log_debug(format!(
        "search_context: done elapsed={}ms",
        started.elapsed().as_millis()
      ));
      Ok(text)
    }
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
/// 负责校验参数、调用远端增强服务，并通过 UI 让用户确认返回内容。
fn handle_enhance_prompt(
  args: Option<serde_json::Value>,
  config: &Arc<Config>,
  runtime: &Arc<Runtime>,
) -> Result<String, String> {
  log_debug("enhance_prompt: start".to_string());
  let args = args.unwrap_or_else(|| serde_json::json!({}));
  let prompt = args.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
  let history = args
    .get("conversation_history")
    .and_then(|v| v.as_str())
    .unwrap_or("");

  if prompt.is_empty() {
    return Err("Missing required parameter: prompt".to_string());
  }
  if history.is_empty() {
    return Err("Missing required parameter: conversation_history".to_string());
  }

  log_debug("enhance_prompt: building index manager".to_string());
  let project_root_path = args
    .get("project_root_path")
    .and_then(|v| v.as_str())
    .map(PathBuf::from)
    .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

  let manager = IndexManager::new(
    project_root_path,
    config.base_url.clone(),
    config.token.clone(),
    config.text_extensions.clone(),
    config.max_lines_per_blob,
    config.exclude_patterns.clone(),
  )
  .map_err(|e| format!("Error: {e}"))?;

  let enhancer = PromptEnhancer::new(manager, config.base_url.clone(), config.token.clone())
    .map_err(|e| format!("Error: {e}"))?;

  // UI 等待时间主要用于用户交互，接口超时用于避免网络请求卡死。
  let ui_timeout = Duration::from_secs(8 * 60);
  let enhance_timeout = Duration::from_secs(90);

  if is_headless_mode() {
    log_debug("enhance_prompt: headless mode enabled".to_string());
    let current_prompt = match runtime.block_on(async {
      timeout(enhance_timeout, enhancer.enhance(prompt, history)).await
    }) {
      Ok(Ok(text)) => text,
      Ok(Err(err)) => {
        if err.to_lowercase().contains("timeout") {
          return Ok(format!(
            "Enhancement timed out (8 minutes). Using original prompt: {}",
            prompt
          ));
        }
        return Err(err);
      }
      Err(_) => {
        log_debug("enhance_prompt: api timeout".to_string());
        return Ok(format!("Enhancement timed out (90 seconds). Using original prompt: {}", prompt));
      }
    };

    let continue_cb: ContinueCallback = Arc::new(|current: String| Ok(current));
    log_debug("enhance_prompt: opening ui".to_string());
    return match run_prompt_session(&current_prompt, ui_timeout, continue_cb, false) {
      SessionAction::UseEnhanced(content) => {
        log_debug("enhance_prompt: ui action=use_enhanced".to_string());
        Ok(content)
      }
      SessionAction::UseOriginal => {
        log_debug("enhance_prompt: ui action=use_original".to_string());
        Ok(prompt.to_string())
      }
      SessionAction::EndConversation => {
        log_debug("enhance_prompt: ui action=end_conversation".to_string());
        Ok("__END_CONVERSATION__".to_string())
      }
      SessionAction::Timeout => {
        log_debug("enhance_prompt: ui action=timeout, fallback=enhanced".to_string());
        Ok(current_prompt.clone())
      }
    };
  }

  let enhancer = Arc::new(enhancer);
  let runtime = runtime.clone();
  let history = history.to_string();
  let continue_cb: ContinueCallback = Arc::new(move |current: String| {
    log_debug("enhance_prompt: continue requested".to_string());
    let enhancer = enhancer.clone();
    let history = history.clone();
    runtime.block_on(async move {
      match timeout(enhance_timeout, enhancer.enhance(&current, &history)).await {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(err)) => Err(err),
        Err(_) => Err("Enhancement timed out (90 seconds).".to_string()),
      }
    })
  });

  log_debug("enhance_prompt: opening ui".to_string());
  match run_prompt_session(prompt, ui_timeout, continue_cb, true) {
    SessionAction::UseEnhanced(content) => {
      log_debug("enhance_prompt: ui action=use_enhanced".to_string());
      Ok(content)
    }
    SessionAction::UseOriginal => {
      log_debug("enhance_prompt: ui action=use_original".to_string());
      Ok(prompt.to_string())
    }
    SessionAction::EndConversation => {
      log_debug("enhance_prompt: ui action=end_conversation".to_string());
      Ok("__END_CONVERSATION__".to_string())
    }
    SessionAction::Timeout => {
      log_debug("enhance_prompt: ui action=timeout, fallback=original".to_string());
      Ok(prompt.to_string())
    }
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
