use thiserror::Error;

pub type Result<T, E = CoreError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid {kind}: {value}")]
    Parse { kind: &'static str, value: String },

    #[error("editorial policy blocked publication: {0} violation(s)")]
    PolicyBlocked(usize),

    #[error("invalid configuration for {key}: {reason}")]
    Config { key: String, reason: String },

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl CoreError {
    pub fn parse(kind: &'static str, value: impl Into<String>) -> Self {
        Self::Parse { kind, value: value.into() }
    }

    pub fn config(key: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Config { key: key.into(), reason: reason.into() }
    }
}
