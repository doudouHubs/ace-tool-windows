use std::fs;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
pub fn detect_project_root() -> PathBuf {
  let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
  let mut current = cwd.clone();

  loop {
    let ace_dir = current.join(".ace-tool");
    if ace_dir.is_dir() {
      return current;
    }

    let git_dir = current.join(".git");
    if git_dir.exists() {
      return current;
    }

    if let Some(parent) = current.parent() {
      current = parent.to_path_buf();
    } else {
      break;
    }
  }

  cwd
}

#[allow(dead_code)]
pub fn get_ace_dir(project_root: &Path) -> PathBuf {
  let ace_dir = project_root.join(".ace-tool");
  if !ace_dir.exists() {
    let _ = fs::create_dir_all(&ace_dir);
    let _ = add_to_gitignore(project_root);
  }
  ace_dir
}

#[allow(dead_code)]
pub fn get_index_file_path(project_root: &Path) -> PathBuf {
  get_ace_dir(project_root).join("index.json")
}

#[allow(dead_code)]
pub fn normalize_path(path: &Path) -> String {
  path.to_string_lossy().replace('\\', "/")
}

#[allow(dead_code)]
fn add_to_gitignore(project_root: &Path) -> std::io::Result<()> {
  let gitignore_path = project_root.join(".gitignore");
  let mut content = String::new();
  if gitignore_path.exists() {
    content = fs::read_to_string(&gitignore_path).unwrap_or_default();
    if content.contains(".ace-tool") {
      return Ok(());
    }
  }

  let new_content = if content.is_empty() || content.ends_with('\n') {
    format!("{}{}", content, ".ace-tool/\n")
  } else {
    format!("{}\n.ace-tool/\n", content)
  };

  fs::write(gitignore_path, new_content)?;
  Ok(())
}
