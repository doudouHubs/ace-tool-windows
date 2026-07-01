//! 索引与检索模块入口。

pub mod local_search;
pub mod manager;

/// `search_context` 的提供方类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchProviderKind {
    Remote,
    Local,
}

impl SearchProviderKind {
    /// 解析配置字符串。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "remote" => Some(Self::Remote),
            "local" => Some(Self::Local),
            _ => None,
        }
    }

    /// 输出稳定字符串，用于日志与配置。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Local => "local",
        }
    }
}

/// `local search` 的答案总结模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalSummaryMode {
    Gpt,
    LocalFallbackOnly,
}

impl LocalSummaryMode {
    /// 解析配置字符串。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "gpt" => Some(Self::Gpt),
            "local_fallback_only" => Some(Self::LocalFallbackOnly),
            _ => None,
        }
    }

    /// 输出稳定字符串，用于日志与配置。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gpt => "gpt",
            Self::LocalFallbackOnly => "local_fallback_only",
        }
    }
}

/// `local search` 的索引重建策略。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalIndexRebuildMode {
    Auto,
    ForceFull,
}

impl LocalIndexRebuildMode {
    /// 解析配置字符串。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "force_full" => Some(Self::ForceFull),
            _ => None,
        }
    }

    /// 输出稳定字符串，用于日志与配置。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::ForceFull => "force_full",
        }
    }
}

/// `local search` 的 GPT 语义重排模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalRerankMode {
    Off,
    BroadOnly,
}

impl LocalRerankMode {
    /// 解析配置字符串。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "broad_only" => Some(Self::BroadOnly),
            _ => None,
        }
    }

    /// 输出稳定字符串，用于日志与配置。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::BroadOnly => "broad_only",
        }
    }
}
