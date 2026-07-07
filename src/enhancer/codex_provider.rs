use crate::enhancer::provider::{EnhanceProvider, EnhanceProviderKind};
use crate::logging::log_debug;
use futures::future::BoxFuture;
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::time::sleep;

const MAX_RETRY_ATTEMPTS: usize = 3;
const RETRYABLE_STATUS_CODES: [u16; 4] = [429, 502, 503, 504];

/// 直连 GPT API 的 codex provider。
pub struct CodexProvider {
    api_base: String,
    model: String,
    reasoning_effort: String,
    timeout: Duration,
    client: Client,
}

impl CodexProvider {
    pub fn new(
        api_base: String,
        api_key: String,
        model: String,
        reasoning_effort: String,
        timeout_sec: u64,
    ) -> Result<Self, String> {
        let api_base = api_base.trim().trim_end_matches('/').to_string();
        if api_base.is_empty() {
            return Err("Codex API base URL is required.".to_string());
        }

        let api_key = api_key.trim().to_string();
        if api_key.is_empty() {
            return Err("Codex API key is required.".to_string());
        }

        let model = model.trim().to_string();
        if model.is_empty() {
            return Err("Codex model is required.".to_string());
        }

        let reasoning_effort = reasoning_effort.trim().to_ascii_lowercase();
        if !matches!(reasoning_effort.as_str(), "low" | "medium" | "high") {
            return Err(format!(
                "Invalid Codex reasoning effort: {} (expected low|medium|high)",
                reasoning_effort
            ));
        }

        let mut headers = HeaderMap::new();
        let auth_header = format!("Bearer {}", api_key);
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth_header).map_err(|e| e.to_string())?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let timeout = Duration::from_secs(timeout_sec.max(10));
        let client = Client::builder()
            .default_headers(headers)
            .timeout(timeout)
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            api_base,
            model,
            reasoning_effort,
            timeout,
            client,
        })
    }

    async fn enhance_once(&self, prompt: &str, history: &str) -> Result<String, String> {
        let (system_prompt, user_prompt) = build_codex_messages(prompt, history);
        let payload = build_chat_completion_payload(
            &self.model,
            &self.reasoning_effort,
            &system_prompt,
            &user_prompt,
        );
        let url = format!("{}/chat/completions", self.api_base);

        let mut last_error = String::new();

        for attempt in 1..=MAX_RETRY_ATTEMPTS {
            let attempt_result = self
                .send_chat_completion_request(&url, &payload, attempt)
                .await;

            match attempt_result {
                Ok(content) => {
                    let cleaned = clean_codex_output(&content);
                    if cleaned.is_empty() {
                        return Err("Codex API returned empty enhancement result.".to_string());
                    }
                    return Ok(cleaned);
                }
                Err(err) => {
                    last_error = err.message;
                    if !err.retryable || attempt == MAX_RETRY_ATTEMPTS {
                        break;
                    }

                    let delay = err
                        .retry_after
                        .unwrap_or_else(|| backoff_delay_for_attempt(attempt));
                    log_debug(format!(
                        "enhance_prompt: codex retry scheduled attempt={} wait={}ms",
                        attempt + 1,
                        delay.as_millis()
                    ));
                    sleep(delay).await;
                }
            }
        }

        if last_error.is_empty() {
            Err("Codex API request failed after retries.".to_string())
        } else if MAX_RETRY_ATTEMPTS > 1 {
            Err(format!(
                "{} Retried {} times.",
                last_error, MAX_RETRY_ATTEMPTS
            ))
        } else {
            Err(last_error)
        }
    }

    async fn send_chat_completion_request(
        &self,
        url: &str,
        payload: &Value,
        attempt: usize,
    ) -> Result<String, CodexAttemptError> {
        log_debug(format!(
            "enhance_prompt: codex api request attempt={} model={}",
            attempt, self.model
        ));

        let response = self
            .client
            .post(url)
            .json(payload)
            .send()
            .await
            .map_err(|err| self.map_request_error(err))?;

        let status = response.status().as_u16();
        let retry_after = parse_retry_after_seconds(response.headers());
        let response_text = response.text().await.map_err(|err| CodexAttemptError {
            message: format!("Failed to read Codex API response: {err}"),
            retryable: false,
            retry_after: None,
        })?;

        if !(200..=299).contains(&status) {
            return Err(CodexAttemptError {
                message: map_http_error(status, &response_text),
                retryable: is_retryable_status(status),
                retry_after,
            });
        }

        parse_chat_completion_content(&response_text).map_err(|message| CodexAttemptError {
            message,
            retryable: false,
            retry_after: None,
        })
    }

    fn map_request_error(&self, err: reqwest::Error) -> CodexAttemptError {
        if err.is_timeout() {
            return CodexAttemptError {
                message: format!(
                    "Codex API request timed out ({} seconds): {}",
                    self.timeout.as_secs(),
                    err
                ),
                retryable: true,
                retry_after: None,
            };
        }

        if err.is_connect() {
            return CodexAttemptError {
                message: format!("Failed to connect to Codex API: {}", err),
                retryable: true,
                retry_after: None,
            };
        }

        CodexAttemptError {
            message: format!("Codex API request failed: {}", err),
            retryable: false,
            retry_after: None,
        }
    }
}

struct CodexAttemptError {
    message: String,
    retryable: bool,
    retry_after: Option<Duration>,
}

impl EnhanceProvider for CodexProvider {
    fn kind(&self) -> EnhanceProviderKind {
        EnhanceProviderKind::Codex
    }

    fn enhance<'a>(
        &'a self,
        prompt: &'a str,
        conversation_history: &'a str,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move { self.enhance_once(prompt, conversation_history).await })
    }
}

fn build_codex_messages(prompt: &str, history: &str) -> (String, String) {
    let system_prompt = "你是提示词增强助手。请把用户提供的原始提示词改写成一版可以直接交给模型执行的最终提示词。\
\n要求：\
\n1. 保留原始意图，不改变任务目标；结合对话上下文补足必要约束，但不要杜撰项目事实。\
\n2. 信息量不能缩水：增强结果的细节、约束和可执行性必须至少不低于原文；若原文偏简略，应适度扩展为更具体版本。\
\n3. 表达形式要自适应原始语义，不要套固定模板：可用自然分段、小标题或列表，但仅在有助于理解时使用，不强制。\
\n4. 文本应便于浏览：避免超长单段，保持逻辑连贯与重点清晰。\
\n5. 只输出增强后的最终提示词正文，不要输出分析、解释、标题前缀或 markdown 代码块。";

    let user_prompt = format!(
        "原始提示词：\n{}\n\n对话上下文：\n{}\n",
        prompt.trim(),
        history.trim()
    );

    (system_prompt.to_string(), user_prompt)
}

fn build_chat_completion_payload(
    model: &str,
    reasoning_effort: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Value {
    json!({
        "model": model,
        "messages": [
            {
                "role": "system",
                "content": system_prompt
            },
            {
                "role": "user",
                "content": user_prompt
            }
        ],
        "temperature": 0.1,
        "reasoning_effort": reasoning_effort
    })
}

fn parse_chat_completion_content(response_text: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(response_text)
        .map_err(|err| format!("Failed to parse Codex API response JSON: {err}"))?;

    let choices = value
        .get("choices")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "Codex API response missing choices array.".to_string())?;

    let first_choice = choices
        .first()
        .ok_or_else(|| "Codex API response returned no choices.".to_string())?;

    let message = first_choice
        .get("message")
        .ok_or_else(|| "Codex API response missing message field.".to_string())?;

    extract_message_content(message)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| "Codex API response returned empty message content.".to_string())
}

fn extract_message_content(message: &Value) -> Option<String> {
    if let Some(content) = message.get("content").and_then(|value| value.as_str()) {
        return Some(content.to_string());
    }

    let parts = message.get("content").and_then(|value| value.as_array())?;
    let mut text_parts = Vec::new();
    for part in parts {
        if let Some(text) = part.get("text").and_then(|value| value.as_str()) {
            text_parts.push(text.trim().to_string());
        }
    }

    if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join("\n").trim().to_string())
    }
}

fn map_http_error(status: u16, response_text: &str) -> String {
    let detail = sanitize_error_detail(response_text);
    match status {
        401 => format!(
            "Codex API authentication failed (401). Check --codex-api-key / ACE_TOOL_CODEX_API_KEY.{}",
            format_error_detail_suffix(&detail)
        ),
        403 => format!(
            "Codex API access forbidden (403). Check API key permissions or service policy.{}",
            format_error_detail_suffix(&detail)
        ),
        429 => format!(
            "Codex API rate limited (429). Check quota, credits, or retry later.{}",
            format_error_detail_suffix(&detail)
        ),
        503 => format!(
            "Codex API service temporarily unavailable (503).{}",
            format_error_detail_suffix(&detail)
        ),
        500..=599 => format!(
            "Codex API server error ({}).{}",
            status,
            format_error_detail_suffix(&detail)
        ),
        _ => format!(
            "Codex API request failed with status {}.{}",
            status,
            format_error_detail_suffix(&detail)
        ),
    }
}

fn format_error_detail_suffix(detail: &str) -> String {
    if detail.is_empty() {
        String::new()
    } else {
        format!(" Response: {}", detail)
    }
}

fn is_retryable_status(status: u16) -> bool {
    RETRYABLE_STATUS_CODES.contains(&status)
}

fn backoff_delay_for_attempt(attempt: usize) -> Duration {
    match attempt {
        1 => Duration::from_millis(800),
        2 => Duration::from_millis(1600),
        _ => Duration::from_millis(2400),
    }
}

fn parse_retry_after_seconds(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
}

fn clean_codex_output(text: &str) -> String {
    let mut value = text.trim_start_matches('\u{feff}').trim().to_string();

    if value.starts_with("```") {
        let mut lines = value.lines();
        let _ = lines.next();
        let mut body = Vec::new();
        for line in lines {
            if line.trim_start().starts_with("```") {
                break;
            }
            body.push(line);
        }
        value = body.join("\n").trim().to_string();
    }

    let labels = [
        "增强后的提示词：",
        "增强提示词：",
        "优化后的提示词：",
        "enhanced prompt:",
        "enhanced prompt：",
        "prompt:",
    ];

    let lower = value.to_ascii_lowercase();
    for label in labels {
        if lower.starts_with(&label.to_ascii_lowercase()) {
            value = value[label.len()..].trim().to_string();
            break;
        }
    }

    value = strip_leading_chatter_line(&value);
    improve_readability_layout(&value)
}

fn strip_leading_chatter_line(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut lines: Vec<&str> = normalized.lines().collect();

    while lines.len() > 1 {
        let first = lines[0].trim();
        if first.is_empty() {
            lines.remove(0);
            continue;
        }
        if !looks_like_leading_chatter(first) {
            break;
        }
        lines.remove(0);
    }

    lines.join("\n").trim().to_string()
}

fn looks_like_leading_chatter(line: &str) -> bool {
    let trimmed = line.trim_matches(|ch: char| matches!(ch, '*' | '#' | '-' | ' ' | '\t'));
    if trimmed.is_empty() {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    let en_markers = [
        "here is",
        "here's",
        "i have",
        "enhanced prompt",
        "optimized prompt",
        "prompt enhancement",
    ];
    if en_markers.iter().any(|marker| lower.contains(marker)) {
        return true;
    }

    let zh_markers = [
        "下面是",
        "以下是",
        "这里是",
        "我已",
        "我已经",
        "已为你",
        "增强后的提示词",
        "优化后的提示词",
        "已触发",
    ];
    zh_markers.iter().any(|marker| trimmed.contains(marker))
}

fn improve_readability_layout(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines = normalized
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();

    if lines >= 3 {
        return normalized.trim().to_string();
    }

    if normalized.chars().count() < 80 {
        return normalized.trim().to_string();
    }

    let mut candidate = if contains_cjk(&normalized) {
        normalized
            .replace('。', "。\n")
            .replace('！', "！\n")
            .replace('？', "？\n")
            .replace('；', "；\n")
    } else {
        normalized
            .replace(". ", ".\n")
            .replace("! ", "!\n")
            .replace("? ", "?\n")
            .replace("; ", ";\n")
    };

    collapse_blank_lines(&mut candidate);
    candidate.trim().to_string()
}

fn collapse_blank_lines(text: &mut String) {
    let mut result = String::with_capacity(text.len());
    let mut blank_run = 0usize;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run <= 1 {
                result.push('\n');
            }
            continue;
        }

        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(line);
        result.push('\n');
        blank_run = 0;
    }
    *text = result;
}

fn contains_cjk(text: &str) -> bool {
    text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn sanitize_error_detail(text: &str) -> String {
    let compact = text
        .replace('\r', " ")
        .replace('\n', " ")
        .trim()
        .to_string();
    if compact.chars().count() > 300 {
        let short: String = compact.chars().take(300).collect();
        format!("{}...", short)
    } else {
        compact
    }
}

#[cfg(test)]
mod tests {
    use super::{
        backoff_delay_for_attempt, build_chat_completion_payload, clean_codex_output,
        improve_readability_layout, is_retryable_status, map_http_error,
        parse_chat_completion_content, strip_leading_chatter_line,
    };
    use std::time::Duration;

    #[test]
    fn build_payload_uses_model_and_messages() {
        let payload = build_chat_completion_payload("gpt-5.4", "low", "system", "user");
        assert_eq!(
            payload.get("model").and_then(|v| v.as_str()),
            Some("gpt-5.4")
        );
        assert_eq!(
            payload.get("reasoning_effort").and_then(|v| v.as_str()),
            Some("low")
        );
        let messages = payload
            .get("messages")
            .and_then(|v| v.as_array())
            .expect("messages array");
        assert_eq!(messages.len(), 2);
    }

    #[test]
    fn parse_response_reads_string_content() {
        let response =
            r#"{"choices":[{"message":{"content":"增强后的提示词：请实现一个 CLI。"}}]}"#;
        let content = parse_chat_completion_content(response).expect("content");
        assert_eq!(content, "增强后的提示词：请实现一个 CLI。");
    }

    #[test]
    fn parse_response_reads_text_parts() {
        let response =
            r#"{"choices":[{"message":{"content":[{"text":"第一行"},{"text":"第二行"}]}}]}"#;
        let content = parse_chat_completion_content(response).expect("content");
        assert_eq!(content, "第一行\n第二行");
    }

    #[test]
    fn http_error_401_mentions_api_key() {
        let err = map_http_error(401, r#"{"error":"bad key"}"#);
        assert!(err.contains("ACE_TOOL_CODEX_API_KEY"));
    }

    #[test]
    fn retryable_status_codes_are_expected() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(503));
        assert!(!is_retryable_status(401));
    }

    #[test]
    fn backoff_grows_with_attempts() {
        assert_eq!(backoff_delay_for_attempt(1), Duration::from_millis(800));
        assert_eq!(backoff_delay_for_attempt(2), Duration::from_millis(1600));
        assert_eq!(backoff_delay_for_attempt(3), Duration::from_millis(2400));
    }

    #[test]
    fn clean_output_strips_label() {
        let input = "增强后的提示词：请用 Rust 实现一个可复用的日志模块。";
        let output = clean_codex_output(input);
        assert_eq!(output, "请用 Rust 实现一个可复用的日志模块。");
    }

    #[test]
    fn clean_output_unwraps_code_block() {
        let input = "```text\n请实现一个登录接口，并补充错误处理。\n```";
        let output = clean_codex_output(input);
        assert_eq!(output, "请实现一个登录接口，并补充错误处理。");
    }

    #[test]
    fn clean_output_strips_leading_chatter_line() {
        let input =
            "我已经帮你增强好了，下面是更具体的版本：\n请使用 Rust 实现一个 CLI，并补充参数校验。";
        let output = clean_codex_output(input);
        assert_eq!(output, "请使用 Rust 实现一个 CLI，并补充参数校验。");
    }

    #[test]
    fn clean_output_keeps_single_line_prompt() {
        let input = "下面是故障现象，请给排查步骤。";
        let output = strip_leading_chatter_line(input);
        assert_eq!(output, "下面是故障现象，请给排查步骤。");
    }

    #[test]
    fn layout_breaks_long_chinese_paragraph() {
        let input = "请你以中国古典诗歌创作者身份，创作一首仿李白神韵的七言古诗，主题为月下饮酒与远行。在不照搬具体名句的前提下，体现李白式的豪放、飘逸与高远想象。全诗须围绕月下把酒、临江将行的情境展开，整体气质豪放而清逸，情感同时包含旷达胸襟与离别沉思。";
        let output = improve_readability_layout(input);
        assert!(output.contains('\n'));
    }

    #[test]
    fn layout_keeps_structured_text() {
        let input = "【任务目标】\n写一个需求。\n\n【关键约束】\n1) 简洁\n2) 可执行\n\n【输出格式】\n只输出正文。";
        let output = improve_readability_layout(input);
        assert_eq!(output, input);
    }
}
