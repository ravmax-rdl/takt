#[derive(Debug, thiserror::Error)]
pub enum GoogleError {
    #[error("network error: {0}")]
    Network(String),
    #[error("sync token no longer valid (410)")]
    Gone,
    #[error("not found (404)")]
    NotFound,
    #[error("forbidden: {reason}")]
    Forbidden { reason: String },
    #[error("rate limited after retries")]
    RateLimited,
    #[error("authorization was revoked or has expired")]
    InvalidGrant,
    #[error("HTTP {status}: {body}")]
    Http { status: u16, body: String },
    #[error("unexpected response: {0}")]
    Decode(String),
    #[error("sign-in failed: {0}")]
    AuthFlow(String),
    #[error("keychain: {0}")]
    TokenStore(String),
}

impl GoogleError {
    pub(crate) fn from_reqwest(error: reqwest::Error) -> Self {
        GoogleError::Network(error.to_string())
    }
}
