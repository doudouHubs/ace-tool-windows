use encoding_rs::{GB18030, GBK, UTF_8, WINDOWS_1252};
use std::fs;
use std::io;
use std::path::Path;

/// 尝试多种编码读取文件内容，优先选择“最少乱码”的结果。
pub fn read_file_with_encoding(path: &Path) -> io::Result<String> {
  let bytes = fs::read(path)?;
  let encodings = [UTF_8, GBK, GB18030, WINDOWS_1252];

  for enc in encodings {
    let (cow, _, had_errors) = enc.decode(&bytes);
    let content = cow.to_string();
    if content.is_empty() {
      continue;
    }

    let replacement_chars = content.chars().filter(|c| *c == '\u{FFFD}').count();
    let length = content.chars().count().max(1);
    if length < 100 {
      if replacement_chars > 5 {
        continue;
      }
    } else if (replacement_chars as f32 / length as f32) > 0.05 {
      continue;
    }

    if !had_errors {
      return Ok(content);
    }

    if replacement_chars == 0 {
      return Ok(content);
    }
  }

  let (cow, _, _) = UTF_8.decode(&bytes);
  Ok(cow.to_string())
}
