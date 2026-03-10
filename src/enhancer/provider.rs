use futures::future::BoxFuture;

/// 增强提供方类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnhanceProviderKind {
    Remote,
    Codex,
}

impl EnhanceProviderKind {
    /// 将字符串解析为提供方类型。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "remote" => Some(Self::Remote),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    /// 返回标准化名称（用于日志与配置回显）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Codex => "codex",
        }
    }
}

/// 统一增强接口，隔离具体提供方实现。
pub trait EnhanceProvider: Send + Sync {
    fn kind(&self) -> EnhanceProviderKind;

    fn enhance<'a>(
        &'a self,
        prompt: &'a str,
        conversation_history: &'a str,
    ) -> BoxFuture<'a, Result<String, String>>;
}
