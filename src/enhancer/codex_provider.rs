use crate::enhancer::provider::{EnhanceProvider, EnhanceProviderKind};
use crate::mcp::log_debug;
use crate::utils::encoding::decode_bytes_with_fallback;
use futures::future::BoxFuture;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// 本地 Codex 单次增强提供方。
pub struct CodexProvider {
  command: String,
  reasoning_effort: String,
  timeout: Duration,
}

impl CodexProvider {
  pub fn new(command: String, reasoning_effort: String, timeout_sec: u64) -> Self {
    Self {
      command,
      reasoning_effort,
      timeout: Duration::from_secs(timeout_sec.max(10)),
    }
  }

  fn enhance_once(&self, prompt: &str, history: &str) -> Result<String, String> {
    let prompt_text = build_codex_prompt(prompt, history);
    let primary_effort = self.reasoning_effort.trim().to_ascii_lowercase();
    let primary_effort = if primary_effort.is_empty() {
      "low".to_string()
    } else {
      primary_effort
    };

    let primary = self.run_codex_exec(&prompt_text, &primary_effort);
    if primary.is_ok() {
      return primary;
    }

    let primary_err = primary.err().unwrap_or_else(|| "Unknown codex error".to_string());
    if !is_timeout_error(&primary_err) || primary_effort == "none" {
      return Err(primary_err);
    }

    log_debug(format!(
      "enhance_prompt: codex timeout with reasoning={}, retry reasoning=none",
      primary_effort
    ));
    match self.run_codex_exec(&prompt_text, "none") {
      Ok(text) => Ok(text),
      Err(retry_err) => Err(format!("{} Retry with reasoning=none failed: {}", primary_err, retry_err)),
    }
  }

  fn run_codex_exec(&self, prompt_text: &str, reasoning_effort: &str) -> Result<String, String> {
    let output_path = build_temp_path("codex-last");
    let stderr_path = build_temp_path("codex-stderr");
    let command_path = resolve_codex_command(&self.command).unwrap_or_else(|| PathBuf::from(&self.command));

    let stderr_file = File::create(&stderr_path).map_err(|err| format!("Failed to create stderr temp file: {err}"))?;

    log_debug(format!(
      "enhance_prompt: codex exec start timeout={}s reasoning={} cmd={}",
      self.timeout.as_secs(),
      reasoning_effort,
      command_path.display()
    ));

    let mut child = build_codex_command(&command_path, reasoning_effort, &output_path)
      .stdin(Stdio::piped())
      .stdout(Stdio::null())
      .stderr(Stdio::from(stderr_file))
      .spawn()
      .map_err(|err| format!("Failed to start codex command '{}': {err}", command_path.display()))?;

    if let Some(mut stdin) = child.stdin.take() {
      stdin
        .write_all(prompt_text.as_bytes())
        .map_err(|err| format!("Failed to write prompt to codex stdin: {err}"))?;
    }

    let started = Instant::now();
    let status = match wait_child_with_timeout(&mut child, self.timeout) {
      Ok(status) => status,
      Err(wait_err) => {
        let stderr_text = read_text_file(&stderr_path);
        cleanup_temp_files(&[output_path, stderr_path]);
        let detail = sanitize_error_detail(stderr_text.trim());
        if detail.is_empty() {
          return Err(wait_err);
        }
        return Err(format!("{} stderr: {}", wait_err, detail));
      }
    };
    let elapsed = started.elapsed().as_millis();

    let stderr_text = read_text_file(&stderr_path);

    if !status.success() {
      let exit_code = status.code().unwrap_or(-1);
      let detail = sanitize_error_detail(stderr_text.trim());
      log_debug(format!(
        "enhance_prompt: codex exec failed exit={} elapsed={}ms stderr_len={}",
        exit_code,
        elapsed,
        detail.chars().count()
      ));
      cleanup_temp_files(&[output_path, stderr_path]);
      if detail.is_empty() {
        return Err(format!("Codex enhancement failed with exit code {}.", exit_code));
      }
      return Err(format!("Codex enhancement failed (exit={}): {}", exit_code, detail));
    }

    let output_text = read_text_file(&output_path);
    cleanup_temp_files(&[output_path, stderr_path]);

    let cleaned = clean_codex_output(&output_text);
    if cleaned.is_empty() {
      return Err("Codex returned empty enhancement result.".to_string());
    }

    log_debug(format!("enhance_prompt: codex exec done elapsed={}ms", elapsed));
    Ok(cleaned)
  }
}

fn is_timeout_error(err: &str) -> bool {
  err.to_ascii_lowercase().contains("timed out")
}

fn build_codex_command(command_path: &Path, reasoning_effort: &str, output_path: &Path) -> Command {
  let config_arg = format!("model_reasoning_effort='{}'", reasoning_effort.replace('\'', ""));
  let args = [
    "exec".to_string(),
    "-c".to_string(),
    config_arg,
    "-c".to_string(),
    "mcp_servers.mcp-router.enabled=false".to_string(),
    "--color".to_string(),
    "never".to_string(),
    "--output-last-message".to_string(),
    output_path.display().to_string(),
    "--skip-git-repo-check".to_string(),
    "-".to_string(),
  ];

  if is_batch_launcher(command_path) {
    let mut command = Command::new("cmd.exe");
    command.arg("/c").arg(build_cmd_line(command_path, &args));
    return command;
  }

  let mut command = Command::new(command_path);
  for arg in args {
    command.arg(arg);
  }
  command
}

fn resolve_codex_command(command: &str) -> Option<PathBuf> {
  let trimmed = command.trim();
  if trimmed.is_empty() {
    return None;
  }

  let direct_path = PathBuf::from(trimmed);
  if direct_path.is_file() {
    return normalize_codex_path(&direct_path);
  }

  find_in_path(trimmed)
    .and_then(|path| normalize_codex_path(&path))
    .or_else(find_windows_codex_fallback)
}

fn normalize_codex_path(path: &Path) -> Option<PathBuf> {
  if !path.is_file() {
    return None;
  }

  if let Some(sibling) = find_windows_launcher_sibling(path) {
    return normalize_codex_path(&sibling);
  }

  let extension = path
    .extension()
    .and_then(|value| value.to_str())
    .map(|value| value.to_ascii_lowercase())
    .unwrap_or_default();

  if matches!(extension.as_str(), "cmd" | "bat") {
    return resolve_volta_shim(path).or_else(|| Some(path.to_path_buf()));
  }

  Some(path.to_path_buf())
}

fn find_windows_launcher_sibling(path: &Path) -> Option<PathBuf> {
  let extension = path.extension().and_then(|value| value.to_str()).unwrap_or_default();
  if !extension.is_empty() {
    return None;
  }

  let stem = path.file_stem()?.to_str()?;
  let parent = path.parent()?;
  for candidate in [
    parent.join(format!("{}.exe", stem)),
    parent.join(format!("{}.cmd", stem)),
    parent.join(format!("{}.bat", stem)),
  ] {
    if candidate.is_file() {
      return Some(candidate);
    }
  }
  None
}

fn resolve_volta_shim(path: &Path) -> Option<PathBuf> {
  let file_name = path.file_name()?.to_str()?.to_ascii_lowercase();
  if file_name != "codex.cmd" && file_name != "codex.bat" {
    return None;
  }

  let local_app_data = env::var_os("LOCALAPPDATA")?;
  let vendor = PathBuf::from(local_app_data)
    .join("Volta")
    .join("tools")
    .join("image")
    .join("packages")
    .join("@openai")
    .join("codex")
    .join("node_modules")
    .join("@openai")
    .join("codex")
    .join("node_modules")
    .join("@openai")
    .join("codex-win32-x64")
    .join("vendor")
    .join("x86_64-pc-windows-msvc")
    .join("codex")
    .join("codex.exe");

  if vendor.is_file() {
    Some(vendor)
  } else {
    None
  }
}

fn find_in_path(command: &str) -> Option<PathBuf> {
  let has_extension = Path::new(command).extension().is_some();
  let path_var = env::var_os("PATH")?;

  for dir in env::split_paths(&path_var) {
    if !has_extension {
      for candidate in [
        dir.join(format!("{}.exe", command)),
        dir.join(format!("{}.cmd", command)),
        dir.join(format!("{}.bat", command)),
      ] {
        if candidate.is_file() {
          return Some(candidate);
        }
      }
    }

    let exact = dir.join(command);
    if exact.is_file() {
      return Some(exact);
    }
  }

  None
}

fn find_windows_codex_fallback() -> Option<PathBuf> {
  let local_app_data = env::var_os("LOCALAPPDATA");
  let app_data = env::var_os("APPDATA");

  let mut candidates = Vec::new();

  if let Some(local_app_data) = local_app_data {
    let local_root = PathBuf::from(local_app_data);
    candidates.push(local_root.join("Volta").join("bin").join("codex.cmd"));
    candidates.push(
      local_root
        .join("Volta")
        .join("tools")
        .join("image")
        .join("packages")
        .join("@openai")
        .join("codex")
        .join("node_modules")
        .join("@openai")
        .join("codex")
        .join("node_modules")
        .join("@openai")
        .join("codex-win32-x64")
        .join("vendor")
        .join("x86_64-pc-windows-msvc")
        .join("codex")
        .join("codex.exe"),
    );
  }

  if let Some(app_data) = app_data {
    candidates.push(PathBuf::from(app_data).join("npm").join("codex.cmd"));
  }

  candidates.into_iter().find_map(|path| normalize_codex_path(&path))
}

fn is_batch_launcher(path: &Path) -> bool {
  path
    .extension()
    .and_then(|value| value.to_str())
    .map(|value| {
      let lower = value.to_ascii_lowercase();
      lower == "cmd" || lower == "bat"
    })
    .unwrap_or(false)
}

fn build_cmd_line(command_path: &Path, args: &[String]) -> String {
  let mut command_line = quote_cmd_arg(command_path.as_os_str().to_string_lossy().as_ref());
  for arg in args {
    command_line.push(' ');
    command_line.push_str(&quote_cmd_arg(arg));
  }
  command_line
}

fn quote_cmd_arg(value: &str) -> String {
  let escaped = value.replace('"', "\"\"");
  format!("\"{}\"", escaped)
}

impl EnhanceProvider for CodexProvider {
  fn kind(&self) -> EnhanceProviderKind {
    EnhanceProviderKind::Codex
  }

  fn enhance<'a>(&'a self, prompt: &'a str, conversation_history: &'a str) -> BoxFuture<'a, Result<String, String>> {
    Box::pin(async move { self.enhance_once(prompt, conversation_history) })
  }
}

fn wait_child_with_timeout(child: &mut Child, timeout: Duration) -> Result<ExitStatus, String> {
  let started = Instant::now();
  loop {
    match child.try_wait() {
      Ok(Some(status)) => return Ok(status),
      Ok(None) => {
        if started.elapsed() >= timeout {
          let _ = child.kill();
          let _ = child.wait();
          return Err(format!("Enhancement timed out ({} seconds).", timeout.as_secs()));
        }
        thread::sleep(Duration::from_millis(120));
      }
      Err(err) => return Err(format!("Failed to wait codex process: {err}")),
    }
  }
}

fn build_codex_prompt(prompt: &str, history: &str) -> String {
  format!(
    "你是提示词增强助手。请把下面的原始提示词改写成一版可以直接交给模型执行的最终提示词。\n要求：\n1. 保留原始意图，不改变任务目标；结合对话上下文补足必要约束，但不要杜撰项目事实。\n2. 信息量不能缩水：增强结果的细节、约束和可执行性必须至少不低于原文；若原文偏简略，应适度扩展为更具体版本。\n3. 表达形式要自适应原始语义，不要套固定模板：可用自然分段、小标题或列表，但仅在有助于理解时使用，不强制。\n4. 文本应便于浏览：避免超长单段，保持逻辑连贯与重点清晰。\n5. 只输出增强后的最终提示词正文，不要输出分析、解释、标题前缀或 markdown 代码块。\n\n原始提示词：\n{}\n\n对话上下文：\n{}\n",
    prompt.trim(),
    history.trim()
  )
}

fn build_temp_path(prefix: &str) -> PathBuf {
  let tick = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|value| value.as_millis())
    .unwrap_or(0);
  let pid = std::process::id();
  std::env::temp_dir().join(format!("ace-tool-{}-{}-{}.txt", prefix, pid, tick))
}

fn read_text_file(path: &PathBuf) -> String {
  match fs::read(path) {
    Ok(bytes) => decode_bytes_with_fallback(&bytes),
    Err(_) => String::new(),
  }
}

fn cleanup_temp_files(paths: &[PathBuf]) {
  for path in paths {
    let _ = fs::remove_file(path);
  }
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

  improve_readability_layout(&value)
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
  text
    .chars()
    .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))
}

fn sanitize_error_detail(text: &str) -> String {
  let compact = text
    .replace('\r', " ")
    .replace('\n', " ")
    .trim()
    .replace("ace_", "ace_[REDACTED]_");
  if compact.chars().count() > 300 {
    let short: String = compact.chars().take(300).collect();
    format!("{}...", short)
  } else {
    compact
  }
}

#[cfg(test)]
mod tests {
  use super::{clean_codex_output, improve_readability_layout};

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
