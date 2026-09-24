use thiserror::Error;

#[derive(Debug, Error)]
pub enum IgError {
    #[error("invalid session (HTTP {status})")]
    InvalidSession { status: u16 },

    #[error("login required")]
    LoginRequired,

    #[error("challenge required: {message}")]
    ChallengeRequired { message: String },

    #[error("feedback required: {message}")]
    FeedbackRequired { message: String },

    #[error("missing permission for {resource} (HTTP {status})")]
    Forbidden { resource: String, status: u16 },

    #[error("resource not found: {resource} (HTTP {status})")]
    NotFound { resource: String, status: u16 },

    #[error("rate limited; retry after {retry_after_ms}ms")]
    RateLimited {
        retry_after_ms: u64,
        global: bool,
        scope: String,
    },

    #[error("cancelled")]
    Cancelled,

    #[error("Instagram API error: HTTP {status} — {message}")]
    Api { status: u16, message: String },

    #[error("HTTP error: {0}")]
    Http(#[from] wreq::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("invalid sessionid")]
    BadSessionId,

    #[error("{0}")]
    Other(String),
}

pub type IgResult<T> = Result<T, IgError>;
