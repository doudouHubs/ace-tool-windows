use crate::mcp::log_debug;
use std::sync::Arc;
use std::env;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum SessionAction {
  UseEnhanced(String),
  UseOriginal,
  EndConversation,
  Timeout,
}

pub type ContinueCallback = Arc<dyn Fn(String) -> Result<String, String> + Send + Sync>;

pub fn run_prompt_session(
  enhanced_prompt: &str,
  timeout: Duration,
  continue_cb: ContinueCallback,
) -> SessionAction {
  if is_headless_mode() {
    log_debug("enhance_prompt: headless mode enabled".to_string());
    return headless_action(enhanced_prompt);
  }
  super::window::run_prompt_window(enhanced_prompt, timeout, continue_cb)
}

fn is_headless_mode() -> bool {
  env::var("ACE_TOOL_HEADLESS")
    .map(|value| {
      let normalized = value.trim().to_ascii_lowercase();
      matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
    })
    .unwrap_or(false)
}
fn headless_action(enhanced_prompt: &str) -> SessionAction {
  let action = env::var("ACE_TOOL_HEADLESS_ACTION").unwrap_or_else(|_| "enhanced".to_string());
  match action.trim().to_ascii_lowercase().as_str() {
    "end" | "end_conversation" => SessionAction::EndConversation,
    "timeout" => SessionAction::Timeout,
    _ => SessionAction::UseEnhanced(enhanced_prompt.to_string()),
  }
}
