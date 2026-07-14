use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExecutorError {
    #[error("config error: {0}")]
    Config(String),

    #[error("upstream HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("upstream returned status {status}: {body}")]
    UpstreamStatus { status: u16, body: String },

    #[error("auth error: {0}")]
    Auth(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml parse error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("invalid payload: {0}")]
    InvalidPayload(String),

    #[error("capability not supported: {0}")]
    UnsupportedCapability(String),

    #[error("operation timed out after {0} seconds")]
    Timeout(u64),

    #[error("path forbidden: {0}")]
    PathForbidden(String),

    #[error("{0}")]
    EgressBlocked(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, ExecutorError>;

impl From<anyhow::Error> for ExecutorError {
    fn from(value: anyhow::Error) -> Self {
        ExecutorError::Other(value.to_string())
    }
}
