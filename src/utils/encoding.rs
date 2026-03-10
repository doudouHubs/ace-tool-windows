use encoding_rs::{GB18030, GBK, UTF_8, WINDOWS_1252};
use std::fs;
use std::io;
use std::path::Path;

/// 尝试多种编码读取文件内容，优先选择“最少乱码”的结果。
pub fn read_file_with_encoding(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    Ok(decode_bytes_with_fallback(&bytes))
}

/// 尝试将字节数组解码为“最少乱码”的文本。
pub fn decode_bytes_with_fallback(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    let bytes = strip_utf8_bom(bytes);
    let encodings = [UTF_8, GBK, GB18030, WINDOWS_1252];

    for enc in encodings {
        let (cow, _, had_errors) = enc.decode(bytes);
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

        if !had_errors || replacement_chars == 0 {
            return trim_utf8_bom_char(&content);
        }
    }

    let (cow, _, _) = UTF_8.decode(bytes);
    trim_utf8_bom_char(&cow)
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    if bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF {
        &bytes[3..]
    } else {
        bytes
    }
}

fn trim_utf8_bom_char(text: &str) -> String {
    text.trim_start_matches('\u{feff}').to_string()
}
