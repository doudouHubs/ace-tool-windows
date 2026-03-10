use crate::mcp::log_debug;
use std::env;
use std::sync::Arc;
use std::time::Duration;

/// UI 会话最终动作。
#[derive(Debug, Clone)]
pub enum SessionAction {
    UseEnhanced(String),
    UseOriginal,
    EndConversation,
    Timeout,
}

/// “继续增强”按钮回调：传入当前文本，返回新的增强结果。
pub type ContinueCallback = Arc<dyn Fn(String) -> Result<String, String> + Send + Sync>;

/// 启动 UI 会话或 headless 模式。
pub fn run_prompt_session(
    enhanced_prompt: &str,
    timeout: Duration,
    continue_cb: ContinueCallback,
    auto_enhance: bool,
) -> SessionAction {
    if is_headless_mode() {
        log_debug("enhance_prompt: headless mode enabled".to_string());
        return headless_action(enhanced_prompt);
    }
    super::window::run_prompt_window(enhanced_prompt, timeout, continue_cb, auto_enhance)
}

/// 是否启用 headless（无 UI）模式。
pub fn is_headless_mode() -> bool {
    env::var("ACE_TOOL_HEADLESS")
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}
/// headless 模式下的默认动作策略。
fn headless_action(enhanced_prompt: &str) -> SessionAction {
    let action = env::var("ACE_TOOL_HEADLESS_ACTION").unwrap_or_else(|_| "enhanced".to_string());
    match action.trim().to_ascii_lowercase().as_str() {
        "end" | "end_conversation" => SessionAction::EndConversation,
        "timeout" => SessionAction::Timeout,
        _ => SessionAction::UseEnhanced(enhanced_prompt.to_string()),
    }
}
