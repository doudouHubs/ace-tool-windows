use crate::utils::encoding::read_file_with_encoding;
use crate::utils::ignore::load_gitignore;
use ignore::gitignore::Gitignore;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use reqwest::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::time::sleep;
use walkdir::WalkDir;

/// 单个 blob 的最大字节数，过大会被跳过。
const MAX_BLOB_SIZE: usize = 500 * 1024;
/// 单次上传批次的最大字节数。
const MAX_BATCH_SIZE: usize = 5 * 1024 * 1024;

/// 可上传的文件片段（按行切分）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct Blob {
  pub path: String,
  pub content: String,
}

/// 索引结果与统计信息的返回结构。
#[derive(Clone, Debug)]
pub struct IndexResult {
  pub status: String,
  pub message: String,
  #[allow(dead_code)]
  pub stats: Option<IndexStats>,
}

/// 索引统计信息（用于调试或前端展示）。
#[derive(Clone, Debug)]
pub struct IndexStats {
  #[allow(dead_code)]
  pub total_blobs: usize,
  #[allow(dead_code)]
  pub existing_blobs: usize,
  #[allow(dead_code)]
  pub new_blobs: usize,
  #[allow(dead_code)]
  pub failed_batches: Option<usize>,
}

/// 上传策略：按规模动态调整批次与并发。
#[derive(Clone, Debug)]
struct UploadStrategy {
  batch_size: usize,
  concurrency: usize,
  timeout_ms: u64,
  #[allow(dead_code)]
  scale_name: &'static str,
}

/// 索引与检索的核心管理器。
pub struct IndexManager {
  project_root: PathBuf,
  base_url: String,
  #[allow(dead_code)]
  token: String,
  text_extensions: HashSet<String>,
  max_lines_per_blob: usize,
  exclude_patterns: Vec<String>,
  index_file_path: PathBuf,
  client: Client,
}

impl IndexManager {
  /// 初始化索引管理器并创建 HTTP 客户端。
  pub fn new(
    project_root: PathBuf,
    base_url: String,
    token: String,
    text_extensions: HashSet<String>,
    max_lines_per_blob: usize,
    exclude_patterns: Vec<String>,
  ) -> Result<Self, String> {
    let mut headers = HeaderMap::new();
    let auth_header = format!("Bearer {}", token);
    headers.insert(
      AUTHORIZATION,
      HeaderValue::from_str(&auth_header).map_err(|e| e.to_string())?,
    );

    let client = Client::builder()
      .default_headers(headers)
      .timeout(Duration::from_secs(30))
      .build()
      .map_err(|e| e.to_string())?;

    let index_file_path = project_root.join(".ace-tool").join("index.json");

    Ok(Self {
      project_root,
      base_url: base_url.trim_end_matches('/').to_string(),
      token,
      text_extensions,
      max_lines_per_blob,
      exclude_patterns,
      index_file_path,
      client,
    })
  }

  /// 入口方法：先索引，再调用检索 API。
  pub async fn search_context(&self, query: &str) -> String {
    let index_result = self.index_project().await;
    if index_result.status == "error" {
      return format!("Error: Failed to index project. {}", index_result.message);
    }

    let blob_names = self.load_index();
    if blob_names.is_empty() {
      return "Error: No blobs found after indexing.".to_string();
    }

    let payload = json!({
      "information_request": query,
      "blobs": {
        "checkpoint_id": null,
        "added_blobs": blob_names,
        "deleted_blobs": []
      },
      "dialog": [],
      "max_output_length": 0,
      "disable_codebase_retrieval": false,
      "enable_commit_retrieval": false
    });

    let url = format!("{}/agents/codebase-retrieval", self.base_url);
    let result: Result<Value, String> = self
      .retry_request(|| async {
        let response = self.client.post(&url).json(&payload).timeout(Duration::from_secs(60)).send().await?;
        let response = response.error_for_status()?;
        let json = response.json::<Value>().await?;
        Ok(json)
      })
      .await;

    match result {
      Ok(value) => {
        let formatted = value.get("formatted_retrieval").and_then(|v| v.as_str()).unwrap_or("");
        if formatted.is_empty() {
          "No relevant code context found for your query.".to_string()
        } else {
          formatted.to_string()
        }
      }
      Err(err) => format!("Error: {}", err),
    }
  }

  /// 索引整个项目并上传新增 blob。
  pub async fn index_project(&self) -> IndexResult {
    let blobs = self.collect_files().await;
    if blobs.is_empty() {
      return IndexResult {
        status: "error".to_string(),
        message: "No text files found in project".to_string(),
        stats: None,
      };
    }

    let existing_blob_names: HashSet<String> = self.load_index().into_iter().collect();
    let mut blob_hash_map = HashMap::new();
    for blob in &blobs {
      let blob_hash = calculate_blob_name(&blob.path, &blob.content);
      blob_hash_map.insert(blob_hash, blob.clone());
    }

    let all_hashes: HashSet<String> = blob_hash_map.keys().cloned().collect();
    let existing_hashes: HashSet<String> = all_hashes
      .iter()
      .filter(|hash| existing_blob_names.contains(*hash))
      .cloned()
      .collect();
    let new_hashes: Vec<String> = all_hashes
      .iter()
      .filter(|hash| !existing_blob_names.contains(*hash))
      .cloned()
      .collect();

    let blobs_to_upload: Vec<Blob> = new_hashes
      .iter()
      .filter_map(|hash| blob_hash_map.get(hash).cloned())
      .collect();

    let mut uploaded_blob_names: Vec<String> = Vec::new();
    let mut fatal_error: Option<String> = None;

    if !blobs_to_upload.is_empty() {
      let strategy = get_upload_strategy(blobs_to_upload.len());
      let mut pending = blobs_to_upload;
      let mut current_batch_size = strategy.batch_size;
      let max_retry_rounds = 3;
      let mut retry_round = 0;
      let mut total_batch_idx = 0usize;

      while !pending.is_empty() && retry_round < max_retry_rounds && fatal_error.is_none() {
        if retry_round > 0 {
          current_batch_size = std::cmp::max(5, current_batch_size / 2);
        }

        let mut batches: Vec<Vec<Blob>> = Vec::new();
        let mut i = 0;
        while i < pending.len() {
          let end = std::cmp::min(i + current_batch_size, pending.len());
          batches.push(pending[i..end].to_vec());
          i = end;
        }

        let mut failed_in_round: Vec<Blob> = Vec::new();
        let mut idx = 0;
        while idx < batches.len() {
          let concurrent = &batches[idx..std::cmp::min(idx + strategy.concurrency, batches.len())];
          let mut handles = Vec::new();
          for batch in concurrent.iter() {
            total_batch_idx += 1;
            let batch_clone = batch.clone();
            let timeout = strategy.timeout_ms;
            let url = format!("{}/batch-upload", self.base_url);
            let client = self.client.clone();
            handles.push(tokio::spawn(async move {
              upload_batch(client, url, batch_clone, total_batch_idx, timeout).await
            }));
          }

          for handle in handles {
            match handle.await {
              Ok(Ok(result)) => {
                if result.success {
                  uploaded_blob_names.extend(result.blob_names);
                } else if result.fatal {
                  fatal_error = Some(result.error.unwrap_or_else(|| "fatal error".to_string()));
                } else {
                  failed_in_round.extend(result.failed_blobs);
                }
              }
              Ok(Err(err)) => {
                failed_in_round.extend(err.failed_blobs);
              }
              Err(_) => {
                // task join error - treat as retryable
              }
            }
          }

          if !uploaded_blob_names.is_empty() {
            let _ = self.save_index([
              existing_hashes.iter().cloned().collect::<Vec<_>>(),
              uploaded_blob_names.clone(),
            ].concat());
          }

          if fatal_error.is_some() {
            break;
          }

          idx += strategy.concurrency;
        }

        pending = failed_in_round;
        retry_round += 1;
      }

      let final_failed = pending.len();
      let all_blob_names = [
        existing_hashes.iter().cloned().collect::<Vec<_>>(),
        uploaded_blob_names.clone(),
      ]
      .concat();
      let _ = self.save_index(all_blob_names.clone());

      if let Some(err) = fatal_error {
        if !uploaded_blob_names.is_empty() {
          return IndexResult {
            status: "partial_success".to_string(),
            message: format!(
              "部分索引成功: {} 个文件块已保存 (已有: {}, 新增: {})。错误: {}。请修复问题后重试，已完成的部分会被保留。",
              all_blob_names.len(),
              existing_hashes.len(),
              uploaded_blob_names.len(),
              err
            ),
            stats: Some(IndexStats {
              total_blobs: all_blob_names.len(),
              existing_blobs: existing_hashes.len(),
              new_blobs: uploaded_blob_names.len(),
              failed_batches: Some(final_failed),
            }),
          };
        }

        if !existing_hashes.is_empty() {
          return IndexResult {
            status: "partial_success".to_string(),
            message: format!(
              "本次上传失败，但保留了 {} 个已有索引。错误: {}。请修复问题后重试。",
              existing_hashes.len(),
              err
            ),
            stats: Some(IndexStats {
              total_blobs: existing_hashes.len(),
              existing_blobs: existing_hashes.len(),
              new_blobs: 0,
              failed_batches: Some(final_failed),
            }),
          };
        }

        return IndexResult {
          status: "error".to_string(),
          message: err,
          stats: None,
        };
      }

      if final_failed > 0 && uploaded_blob_names.is_empty() && existing_hashes.is_empty() {
        return IndexResult {
          status: "error".to_string(),
          message: "所有文件上传失败，请检查网络连接和服务配置".to_string(),
          stats: None,
        };
      }
    }

    let all_blob_names = [
      existing_hashes.iter().cloned().collect::<Vec<_>>(),
      uploaded_blob_names.clone(),
    ]
    .concat();
    let _ = self.save_index(all_blob_names.clone());
    IndexResult {
      status: "success".to_string(),
      message: format!(
        "Indexed {} blobs (existing: {}, new: {})",
        all_blob_names.len(),
        existing_hashes.len(),
        uploaded_blob_names.len()
      ),
      stats: Some(IndexStats {
        total_blobs: all_blob_names.len(),
        existing_blobs: existing_hashes.len(),
        new_blobs: uploaded_blob_names.len(),
        failed_batches: None,
      }),
    }
  }

  /// 遍历项目并切分为可上传的 blob。
  async fn collect_files(&self) -> Vec<Blob> {
    let gitignore = load_gitignore(&self.project_root);
    let mut blobs = Vec::new();

    let mut walker = WalkDir::new(&self.project_root).into_iter();
    while let Some(entry) = walker.next() {
      let entry = match entry {
        Ok(item) => item,
        Err(_) => continue,
      };
      let path = entry.path();
      let is_dir = entry.file_type().is_dir();

      if should_exclude_path(path, is_dir, &self.project_root, gitignore.as_ref(), &self.exclude_patterns) {
        if is_dir {
          walker.skip_current_dir();
        }
        continue;
      }

      if is_dir {
        continue;
      }

      let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
      let ext = if ext.is_empty() { String::new() } else { format!(".{}", ext.to_lowercase()) };
      if !self.text_extensions.contains(&ext) {
        continue;
      }

      let relative = match path.strip_prefix(&self.project_root) {
        Ok(rel) => rel,
        Err(_) => continue,
      };
      let relative_path = normalize_path(relative);

      let content = match read_file_with_encoding(path) {
        Ok(data) => data,
        Err(_) => continue,
      };

      if is_binary_content(&content) {
        continue;
      }

      let clean_content = sanitize_content(&content);
      if clean_content.as_bytes().len() > MAX_BLOB_SIZE {
        continue;
      }

      let file_blobs = split_file_content(&relative_path, &clean_content, self.max_lines_per_blob);
      blobs.extend(file_blobs);
    }

    blobs
  }

  /// 从 `.ace-tool/index.json` 读取已上传的 blob 名称。
  pub fn load_index(&self) -> Vec<String> {
    if !self.index_file_path.exists() {
      return Vec::new();
    }
    let content = fs::read_to_string(&self.index_file_path).unwrap_or_default();
    serde_json::from_str::<Vec<String>>(&content).unwrap_or_default()
  }

  /// 保存 blob 名称索引到本地文件。
  fn save_index(&self, blob_names: Vec<String>) -> Result<(), String> {
    if let Some(parent) = self.index_file_path.parent() {
      let _ = fs::create_dir_all(parent);
    }
    let content = serde_json::to_string_pretty(&blob_names).map_err(|e| e.to_string())?;
    fs::write(&self.index_file_path, content).map_err(|e| e.to_string())?;
    Ok(())
  }

  /// 对网络请求进行指数退避重试。
  async fn retry_request<F, Fut, T>(&self, mut operation: F) -> Result<T, String>
  where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, reqwest::Error>>,
  {
    let max_retries = 3;
    let mut last_error: Option<reqwest::Error> = None;

    for attempt in 0..max_retries {
      match operation().await {
        Ok(value) => return Ok(value),
        Err(err) => {
          if let Some(status) = err.status() {
            if status.as_u16() == 401 {
              return Err("Token 已失效或无效，请更新 ACE_TOKEN 环境变量".to_string());
            }
            if status.as_u16() == 403 {
              return Err("访问被拒绝，Token 可能已被官方禁用，请联系服务提供商".to_string());
            }
            if status.as_u16() == 502 {
              return Err("服务器网关错误 (502)，ACE 服务可能暂时不可用，请稍后重试".to_string());
            }
            if status.as_u16() == 503 {
              return Err("服务暂时不可用 (503)，ACE 服务可能正在维护，请稍后重试".to_string());
            }
            if status.as_u16() == 504 {
              return Err("网关超时 (504)，服务器响应过慢，请稍后重试".to_string());
            }
          }

          let message = err.to_string();
          if message.contains("certificate") || message.contains("tls") {
            return Err("SSL 证书验证失败，请检查 ACE_BASE_URL 配置是否正确，或联系服务提供商".to_string());
          }

          last_error = Some(err);
          if attempt == max_retries - 1 {
            break;
          }
          let wait = 1000u64 * 2u64.pow(attempt as u32);
          sleep(Duration::from_millis(wait)).await;
        }
      }
    }

    let err = last_error.map(|e| e.to_string()).unwrap_or_else(|| "All retries failed".to_string());
    Err(err)
  }
}

/// 单批上传的结果结构。
#[derive(Debug)]
struct UploadBatchResult {
  success: bool,
  blob_names: Vec<String>,
  failed_blobs: Vec<Blob>,
  error: Option<String>,
  fatal: bool,
}

/// 上传一个批次，并将失败项返回用于重试。
async fn upload_batch(
  client: Client,
  url: String,
  batch: Vec<Blob>,
  _batch_idx: usize,
  timeout_ms: u64,
) -> Result<UploadBatchResult, UploadBatchResult> {
  let batch_size = batch
    .iter()
    .map(|blob| blob.content.as_bytes().len())
    .sum::<usize>();
  if batch_size > MAX_BATCH_SIZE {
    return Ok(UploadBatchResult {
      success: false,
      blob_names: Vec::new(),
      failed_blobs: batch,
      error: Some("批次过大，需要拆分".to_string()),
      fatal: false,
    });
  }

  let payload = json!({ "blobs": batch });
  let response = client
    .post(&url)
    .json(&payload)
    .timeout(Duration::from_millis(timeout_ms))
    .send()
    .await;

  match response {
    Ok(resp) => {
      let resp = match resp.error_for_status() {
        Ok(r) => r,
        Err(err) => {
          return Err(UploadBatchResult {
            success: false,
            blob_names: Vec::new(),
            failed_blobs: Vec::new(),
            error: Some(err.to_string()),
            fatal: false,
          });
        }
      };
      let json = resp.json::<Value>().await.unwrap_or_else(|_| json!({}));
      let blob_names = json
        .get("blob_names")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_else(Vec::new);

      if blob_names.is_empty() {
        return Ok(UploadBatchResult {
          success: false,
          blob_names: Vec::new(),
          failed_blobs: batch,
          error: Some("服务器返回空结果".to_string()),
          fatal: false,
        });
      }

      Ok(UploadBatchResult {
        success: true,
        blob_names,
        failed_blobs: Vec::new(),
        error: None,
        fatal: false,
      })
    }
    Err(err) => Err(UploadBatchResult {
      success: false,
      blob_names: Vec::new(),
      failed_blobs: batch,
      error: Some(err.to_string()),
      fatal: false,
    }),
  }
}

/// 根据 blob 数量选择上传策略。
fn get_upload_strategy(blob_count: usize) -> UploadStrategy {
  if blob_count < 100 {
    UploadStrategy {
      batch_size: 10,
      concurrency: 1,
      timeout_ms: 30000,
      scale_name: "小型",
    }
  } else if blob_count < 500 {
    UploadStrategy {
      batch_size: 30,
      concurrency: 2,
      timeout_ms: 45000,
      scale_name: "中型",
    }
  } else if blob_count < 2000 {
    UploadStrategy {
      batch_size: 50,
      concurrency: 3,
      timeout_ms: 60000,
      scale_name: "大型",
    }
  } else {
    UploadStrategy {
      batch_size: 70,
      concurrency: 4,
      timeout_ms: 90000,
      scale_name: "超大型",
    }
  }
}

/// 计算 blob 的稳定哈希名（路径 + 内容）。
fn calculate_blob_name(path: &str, content: &str) -> String {
  let mut hasher = Sha256::new();
  hasher.update(path.as_bytes());
  hasher.update(content.as_bytes());
  let digest = hasher.finalize();
  digest.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 清理不可见控制字符，避免服务端解析失败。
fn sanitize_content(content: &str) -> String {
  content
    .chars()
    .filter(|c| {
      let code = *c as u32;
      !(code <= 0x08 || code == 0x0B || code == 0x0C || (0x0E..=0x1F).contains(&code) || code == 0x7F)
    })
    .collect()
}

/// 简单二进制判定：非可见字符比例过高则视为二进制。
fn is_binary_content(content: &str) -> bool {
  let mut non_printable = 0usize;
  for ch in content.chars() {
    let code = ch as u32;
    if (code <= 0x08) || (0x0E..=0x1F).contains(&code) || code == 0x7F {
      non_printable += 1;
    }
  }
  let total = content.chars().count().max(1);
  (non_printable as f32 / total as f32) > 0.1
}

/// 按行切分文件内容，生成多个 blob。
fn split_file_content(path: &str, content: &str, max_lines: usize) -> Vec<Blob> {
  let bytes = content.as_bytes();
  let mut lines: Vec<String> = Vec::new();
  let mut start = 0usize;
  let mut i = 0usize;
  while i < bytes.len() {
    if bytes[i] == b'\n' {
      let end = i + 1;
      lines.push(content[start..end].to_string());
      start = end;
    } else if bytes[i] == b'\r' {
      if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
        let end = i + 2;
        lines.push(content[start..end].to_string());
        start = end;
        i += 1;
      } else {
        let end = i + 1;
        lines.push(content[start..end].to_string());
        start = end;
      }
    }
    i += 1;
  }

  if start < content.len() {
    lines.push(content[start..].to_string());
  }

  let total_lines = lines.len();
  if total_lines <= max_lines {
    return vec![Blob {
      path: path.to_string(),
      content: content.to_string(),
    }];
  }

  let num_chunks = (total_lines + max_lines - 1) / max_lines;
  let mut blobs = Vec::new();
  for chunk_idx in 0..num_chunks {
    let start_line = chunk_idx * max_lines;
    let end_line = std::cmp::min(start_line + max_lines, total_lines);
    let chunk_content = lines[start_line..end_line].join("");
    let chunk_path = format!("{}#chunk{}of{}", path, chunk_idx + 1, num_chunks);
    blobs.push(Blob {
      path: chunk_path,
      content: chunk_content,
    });
  }

  blobs
}

/// 判断路径是否应被忽略（gitignore + 自定义规则）。
fn should_exclude_path(
  path: &Path,
  is_dir: bool,
  project_root: &Path,
  gitignore: Option<&Gitignore>,
  exclude_patterns: &[String],
) -> bool {
  let relative = match path.strip_prefix(project_root) {
    Ok(rel) => rel,
    Err(_) => return false,
  };
  let path_str = normalize_path(relative);
  if let Some(gitignore) = gitignore {
    let matched = gitignore.matched_path_or_any_parents(relative, is_dir);
    if matched.is_ignore() {
      return true;
    }
  }

  let parts: Vec<&str> = path_str.split('/').collect();
  for pattern in exclude_patterns {
    for part in &parts {
      if wildcard_match(part, pattern) {
        return true;
      }
    }
    if wildcard_match(&path_str, pattern) {
      return true;
    }
  }

  false
}

/// 统一路径分隔符为 `/`，便于跨平台比较。
fn normalize_path(path: &Path) -> String {
  path.to_string_lossy().replace('\\', "/")
}

/// 简单通配符匹配（支持 `*` 与 `?`）。
fn wildcard_match(text: &str, pattern: &str) -> bool {
  let t = text.as_bytes();
  let p = pattern.as_bytes();
  let mut ti = 0usize;
  let mut pi = 0usize;
  let mut star_idx: Option<usize> = None;
  let mut match_idx = 0usize;

  while ti < t.len() {
    if pi < p.len() && (p[pi] == b'?' || p[pi] == t[ti]) {
      ti += 1;
      pi += 1;
    } else if pi < p.len() && p[pi] == b'*' {
      star_idx = Some(pi);
      match_idx = ti;
      pi += 1;
    } else if let Some(star) = star_idx {
      pi = star + 1;
      match_idx += 1;
      ti = match_idx;
    } else {
      return false;
    }
  }

  while pi < p.len() && p[pi] == b'*' {
    pi += 1;
  }

  pi == p.len()
}
