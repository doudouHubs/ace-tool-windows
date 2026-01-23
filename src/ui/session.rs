use std::sync::Arc;
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
  super::window::run_prompt_window(enhanced_prompt, timeout, continue_cb)
}
