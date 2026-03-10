use crate::enhancer::provider::{EnhanceProvider, EnhanceProviderKind};
use crate::index::manager::IndexManager;
use futures::future::BoxFuture;
use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::time::Duration;

/// 远端增强提供方，封装与 `/prompt-enhancer` 的交互。
pub struct RemoteProvider {
    base_url: String,
    client: Client,
    index_manager: IndexManager,
}

impl RemoteProvider {
    /// 创建增强器并初始化 HTTP 客户端（含鉴权头）。
    pub fn new(
        index_manager: IndexManager,
        base_url: String,
        token: String,
    ) -> Result<Self, String> {
        let mut headers = HeaderMap::new();
        let auth_header = format!("Bearer {}", token);
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&auth_header).map_err(|e| e.to_string())?,
        );
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;

        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
            index_manager,
        })
    }

    /// 主入口：加载索引并调用增强 API。
    pub async fn enhance_prompt(
        &self,
        original_prompt: &str,
        conversation_history: &str,
    ) -> Result<String, String> {
        let blob_names = self.index_manager.load_index();
        self.call_prompt_enhancer_api(original_prompt, conversation_history, &blob_names)
            .await
            .map(|text| replace_tool_names(&text))
    }

    /// 直接调用远端 prompt enhancer API。
    pub async fn call_prompt_enhancer_api(
        &self,
        original_prompt: &str,
        conversation_history: &str,
        blob_names: &[String],
    ) -> Result<String, String> {
        let chat_history = parse_chat_history(conversation_history);
        // 根据输入语言决定是否强制中文输出。
        let language_guideline = if should_use_chinese(original_prompt) {
            "Please respond in Chinese (Simplified Chinese). 请务必使用中文回复。增强结果必须比原文更具体、更细致，信息量不能减少。表达方式应自适应原始语义，不要套固定模板；可按需要使用分段、小标题或列表，但不强制。以便于浏览、逻辑清晰为目标。"
        } else {
            "Make the enhanced prompt more concrete and detailed than the original. Do not reduce information density. Adapt the format to the original intent instead of forcing a rigid template. Use paragraphs/headings/lists only when they improve readability."
        };

        let payload = json!({
          "nodes": [
            {
              "id": 1,
              "type": 0,
              "text_node": {
                "content": original_prompt
              }
            }
          ],
          "chat_history": chat_history,
          "blobs": {
            "checkpoint_id": null,
            "added_blobs": blob_names,
            "deleted_blobs": []
          },
          "conversation_id": null,
          "model": "claude-sonnet-4-5",
          "mode": "CHAT",
          "user_guided_blobs": [],
          "external_source_ids": [],
          "user_guidelines": language_guideline,
          "workspace_guidelines": "",
          "rules": []
        });

        let url = format!("{}/prompt-enhancer", self.base_url);
        let response = self.client.post(&url).json(&payload).send().await;

        match response {
            Ok(resp) => {
                if let Some(status) = resp.status().as_u16().checked_sub(0) {
                    if status == 401 {
                        return Err("Token 已失效或无效，请检查配置".to_string());
                    }
                    if status == 403 {
                        return Err("访问被拒绝，Token 可能已被禁用".to_string());
                    }
                }
                let resp = resp.error_for_status().map_err(|e| e.to_string())?;
                let json = resp.json::<Value>().await.map_err(|e| e.to_string())?;
                let text = json.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if text.is_empty() {
                    return Err("Prompt enhancer API returned empty result".to_string());
                }
                Ok(text.to_string())
            }
            Err(err) => {
                let msg = err.to_string();
                if msg.contains("401") {
                    return Err("Token 已失效或无效，请检查配置".to_string());
                }
                if msg.contains("403") {
                    return Err("访问被拒绝，Token 可能已被禁用".to_string());
                }
                if msg.contains("ECONNREFUSED") || msg.contains("Connection refused") {
                    return Err("无法连接到服务器，请检查 base-url 配置".to_string());
                }
                Err(format!("Prompt enhancer API 调用失败: {}", msg))
            }
        }
    }
}

impl EnhanceProvider for RemoteProvider {
    fn kind(&self) -> EnhanceProviderKind {
        EnhanceProviderKind::Remote
    }

    fn enhance<'a>(
        &'a self,
        prompt: &'a str,
        conversation_history: &'a str,
    ) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move { self.enhance_prompt(prompt, conversation_history).await })
    }
}

/// 将文本聊天记录解析为 API 所需的 role/content 列表。
fn parse_chat_history(history: &str) -> Vec<Value> {
    let mut chat_history = Vec::new();
    for line in history.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("User:") || trimmed.starts_with("\u{7528}\u{6237}:") {
            chat_history.push(json!({
        "role": "user",
        "content": trimmed.trim_start_matches("User:").trim_start_matches("\u{7528}\u{6237}:").trim()
      }));
        } else if trimmed.starts_with("AI:")
            || trimmed.starts_with("Assistant:")
            || trimmed.starts_with("\u{52a9}\u{624b}:")
        {
            chat_history.push(json!({
              "role": "assistant",
              "content": trimmed
                .trim_start_matches("AI:")
                .trim_start_matches("Assistant:")
                .trim_start_matches("\u{52a9}\u{624b}:")
                .trim()
            }));
        }
    }
    chat_history
}

/// 将远端工具名映射为本地 MCP 工具名。
fn replace_tool_names(text: &str) -> String {
    text.replace("codebase-retrieval", "search_context")
        .replace("codebase_retrieval", "search_context")
}

/// 根据输入文本粗略判断是否应使用中文回复。
fn should_use_chinese(text: &str) -> bool {
    if contains_cjk(text) {
        return true;
    }

    let ascii_words = count_ascii_words(text);
    let mut latin_letters = 0usize;
    let mut total_letters = 0usize;
    for ch in text.chars() {
        if ch.is_ascii_alphabetic() {
            latin_letters += 1;
            total_letters += 1;
        } else if ch.is_alphabetic() {
            total_letters += 1;
        }
    }

    if total_letters == 0 {
        return true;
    }

    let latin_ratio = latin_letters as f32 / total_letters as f32;
    if ascii_words >= 3 && latin_ratio >= 0.85 {
        return false;
    }

    true
}

/// 判断是否包含 CJK 字符。
fn contains_cjk(text: &str) -> bool {
    text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

/// 统计 ASCII 单词数量，用于粗略语言判定。
fn count_ascii_words(text: &str) -> usize {
    let mut count = 0usize;
    let mut in_word = false;
    for ch in text.chars() {
        if ch.is_ascii_alphabetic() {
            if !in_word {
                count += 1;
                in_word = true;
            }
        } else {
            in_word = false;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::{replace_tool_names, should_use_chinese};

    #[test]
    fn replace_tool_name_variants() {
        let input = "use codebase-retrieval then codebase_retrieval";
        let output = replace_tool_names(input);
        assert_eq!(output, "use search_context then search_context");
    }

    #[test]
    fn should_use_chinese_works_for_basic_cases() {
        assert!(should_use_chinese("请帮我优化这个接口的返回结构"));
        assert!(!should_use_chinese(
            "Please optimize this API response shape."
        ));
    }
}
