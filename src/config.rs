use std::collections::HashSet;

#[derive(Clone, Debug)]
pub struct Config {
  pub base_url: String,
  pub token: String,
  #[allow(dead_code)]
  pub batch_size: usize,
  pub max_lines_per_blob: usize,
  pub text_extensions: HashSet<String>,
  pub exclude_patterns: Vec<String>,
  pub enable_log: bool,
}

struct ParsedArgs {
  base_url: Option<String>,
  token: Option<String>,
  enable_log: bool,
}

pub fn init_config() -> Result<Config, String> {
  let args = parse_args();
  let base_url = args.base_url.ok_or_else(|| "Missing required argument: --base-url".to_string())?;
  let token = args.token.ok_or_else(|| "Missing required argument: --token".to_string())?;

  let mut base_url = normalize_base_url(&base_url);
  if base_url.starts_with("http://") {
    let original = base_url.clone();
    base_url = base_url.replacen("http://", "https://", 1);
    println!("Auto converted http:// to https:// ({} -> {})", original, base_url);
  }

  base_url = base_url.trim_end_matches('/').to_string();

  Ok(Config {
    base_url,
    token,
    batch_size: 10,
    max_lines_per_blob: 800,
    text_extensions: default_text_extensions(),
    exclude_patterns: default_exclude_patterns(),
    enable_log: args.enable_log,
  })
}

fn parse_args() -> ParsedArgs {
  let mut result = ParsedArgs {
    base_url: None,
    token: None,
    enable_log: false,
  };

  let mut iter = std::env::args().skip(1);
  while let Some(arg) = iter.next() {
    match arg.as_str() {
      "--base-url" => {
        if let Some(value) = iter.next() {
          result.base_url = Some(value);
        }
      }
      "--token" => {
        if let Some(value) = iter.next() {
          result.token = Some(value);
        }
      }
      "--enable-log" => {
        result.enable_log = true;
      }
      _ => {}
    }
  }

  result
}

fn normalize_base_url(value: &str) -> String {
  if value.starts_with("http://") || value.starts_with("https://") {
    value.to_string()
  } else {
    format!("https://{}", value)
  }
}

fn default_text_extensions() -> HashSet<String> {
  let list = [
    ".py", ".js", ".ts", ".jsx", ".tsx", ".mjs", ".cjs",
    ".java", ".go", ".rs", ".cpp", ".c", ".cc",
    ".h", ".hpp", ".hxx", ".cs", ".rb", ".php",
    ".swift", ".kt", ".kts", ".scala", ".clj", ".cljs",
    ".lua", ".dart", ".m", ".mm", ".pl", ".pm",
    ".r", ".R", ".jl", ".ex", ".exs", ".erl",
    ".hs", ".zig", ".v", ".nim", ".f90", ".f95",
    ".groovy", ".gradle", ".sol", ".move",
    ".md", ".mdx", ".txt", ".json", ".jsonc", ".json5",
    ".yaml", ".yml", ".toml", ".xml", ".ini", ".conf",
    ".cfg", ".properties", ".env.example", ".editorconfig",
    ".html", ".htm", ".css", ".scss", ".sass", ".less", ".styl",
    ".vue", ".svelte", ".astro",
    ".ejs", ".hbs", ".pug", ".jade", ".jinja", ".jinja2",
    ".erb", ".liquid", ".twig", ".mustache", ".njk",
    ".sql", ".sh", ".bash", ".zsh", ".fish",
    ".ps1", ".psm1", ".bat", ".cmd",
    ".makefile", ".mk", ".cmake",
    ".graphql", ".gql", ".proto", ".prisma",
    ".csv", ".tsv",
    ".rst", ".adoc", ".tex", ".org",
    ".dockerfile", ".containerfile",
    ".vim", ".el", ".rkt",
  ];
  list.iter().map(|s| s.to_string()).collect()
}

fn default_exclude_patterns() -> Vec<String> {
  vec![
    ".venv", "venv", ".env", "env", "node_modules",
    "vendor", ".pnpm", ".yarn", "bower_components",
    ".git", ".svn", ".hg", ".gitmodules",
    "__pycache__", ".pytest_cache", ".mypy_cache",
    ".tox", ".eggs", "*.egg-info", ".ruff_cache",
    "dist", "build", "target", "out", "bin", "obj",
    ".next", ".nuxt", ".output", ".vercel", ".netlify",
    ".turbo", ".parcel-cache", ".cache", ".temp", ".tmp",
    "coverage", ".nyc_output", "htmlcov",
    ".idea", ".vscode", ".vs", "*.swp", "*.swo",
    ".DS_Store", "Thumbs.db", "desktop.ini",
    "*.pyc", "*.pyo", "*.pyd", "*.so", "*.dll", "*.dylib",
    "*.exe", "*.o", "*.obj", "*.class", "*.jar", "*.war",
    "*.min.js", "*.min.css", "*.bundle.js", "*.chunk.js",
    "*.map", "*.gz", "*.zip", "*.tar", "*.rar",
    "package-lock.json", "yarn.lock", "pnpm-lock.yaml",
    "Gemfile.lock", "poetry.lock", "Cargo.lock", "composer.lock",
    "*.log", "logs", "tmp", "temp",
    "*.png", "*.jpg", "*.jpeg", "*.gif", "*.ico", "*.svg",
    "*.mp3", "*.mp4", "*.wav", "*.avi", "*.mov",
    "*.pdf", "*.doc", "*.docx", "*.xls", "*.xlsx",
    "*.woff", "*.woff2", "*.ttf", "*.eot", "*.otf",
    "*.db", "*.sqlite", "*.sqlite3",
    ".ace-tool",
  ]
  .into_iter()
  .map(|s| s.to_string())
  .collect()
}
