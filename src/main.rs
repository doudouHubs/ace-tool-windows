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
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::time::timeout;
use ui::session::{run_prompt_session, ContinueCallback, SessionAction};

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

fn handle_search_context(
  args: Option<serde_json::Value>,
  config: &Arc<Config>,
  runtime: &Arc<Runtime>,
) -> Result<String, String> {
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

  let result = runtime.block_on(manager.search_context(query));
  Ok(result)
}

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

  log_debug("enhance_prompt: calling enhancer".to_string());
  let ui_timeout = Duration::from_secs(8 * 60);
  let enhance_timeout = Duration::from_secs(90);
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
  log_debug("enhance_prompt: enhancer returned".to_string());

  let enhancer = Arc::new(enhancer);
  let runtime = runtime.clone();
  let history = history.to_string();
  let continue_cb: ContinueCallback = Arc::new(move |current: String| {
    log_debug("enhance_prompt: continue requested".to_string());
    let blobs = enhancer.load_blob_names();
    runtime.block_on(enhancer.call_prompt_enhancer_api(&current, &history, &blobs))
  });

  log_debug("enhance_prompt: opening ui".to_string());
  match run_prompt_session(&current_prompt, ui_timeout, continue_cb) {
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
  }
}

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
