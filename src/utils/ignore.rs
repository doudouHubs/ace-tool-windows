use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

pub fn load_gitignore(project_root: &Path) -> Option<Gitignore> {
  let gitignore_path = project_root.join(".gitignore");
  if !gitignore_path.exists() {
    return None;
  }

  let mut builder = GitignoreBuilder::new(project_root);
  builder.add(gitignore_path);
  builder.build().ok()
}
