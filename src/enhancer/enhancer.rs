use crate::index::manager::IndexManager;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

pub struct PromptEnhancer {
  base_url: String,
  client: Client,
  index_manager: IndexManager,
}

impl PromptEnhancer {
  pub fn new(index_manager: IndexManager, base_url: String, token: String) -> Result<Self, String> {
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

  pub async fn enhance(&self, original_prompt: &str, conversation_history: &str) -> Result<String, String> {
    let blob_names = self.index_manager.load_index();
    self.call_prompt_enhancer_api(original_prompt, conversation_history, &blob_names)
      .await
      .map(|text| replace_tool_names(&text))
  }

  pub fn load_blob_names(&self) -> Vec<String> {
    self.index_manager.load_index()
  }

  pub async fn call_prompt_enhancer_api(
    &self,
    original_prompt: &str,
    conversation_history: &str,
    blob_names: &[String],
  ) -> Result<String, String> {
    let chat_history = parse_chat_history(conversation_history);
    let language_guideline = if should_use_chinese(original_prompt) {
      "Please respond in Chinese (Simplified Chinese). 请务必使用中文回复。"
    } else {
      ""
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

fn parse_chat_history(history: &str) -> Vec<Value> {
  let mut chat_history = Vec::new();
  for line in history.lines() {
    let trimmed = line.trim();
    if trimmed.starts_with("User:") || trimmed.starts_with("\u{7528}\u{6237}:") {
      chat_history.push(json!({
        "role": "user",
        "content": trimmed.trim_start_matches("User:").trim_start_matches("\u{7528}\u{6237}:").trim()
      }));
    } else if trimmed.starts_with("AI:") || trimmed.starts_with("Assistant:") || trimmed.starts_with("\u{52a9}\u{624b}:") {
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

fn replace_tool_names(text: &str) -> String {
  text.replace("codebase-retrieval", "search_context")
    .replace("codebase_retrieval", "search_context")
}

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

fn contains_cjk(text: &str) -> bool {
  text
    .chars()
    .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

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
