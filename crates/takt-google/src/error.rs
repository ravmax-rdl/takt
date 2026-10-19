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
    /// `body` is kept whole for callers; `Display` shows only Google's
    /// message, since the text reaches the UI.
    #[error("HTTP {status}: {}", summarize_body(body))]
    Http { status: u16, body: String },
    #[error("unexpected response: {0}")]
    Decode(String),
    #[error("sign-in failed: {0}")]
    AuthFlow(String),
    #[error("keychain: {0}")]
    TokenStore(String),
}

impl GoogleError {
    /// An unexpected HTTP status. The full body is logged at debug level;
    /// the error's `Display` keeps only a short summary of it.
    pub(crate) fn http(status: u16, body: String) -> Self {
        tracing::debug!(status, %body, "Google returned an error response");
        GoogleError::Http { status, body }
    }

    pub(crate) fn from_reqwest(error: reqwest::Error) -> Self {
        // The URL can carry query tokens (sync/page tokens), so drop it, and
        // keep the underlying causes (DNS, TLS, refused) that `Display` hides.
        let error = error.without_url();
        let mut message = error.to_string();
        let mut source = std::error::Error::source(&error);
        while let Some(cause) = source {
            message.push_str(": ");
            message.push_str(&cause.to_string());
            source = cause.source();
        }
        GoogleError::Network(message)
    }
}

/// How much of a non-JSON body `Display` shows.
const SUMMARY_CHARS: usize = 120;

/// Google's own message for an error body, else the body's first line cut to
/// [`SUMMARY_CHARS`]. Handles the API shape (`{"error": {"message": ..}}`)
/// and the OAuth token-endpoint shape (`{"error": "..", "error_description": ..}`).
fn summarize_body(body: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body) {
        let error = value.get("error");
        let message = error
            .and_then(|e| e.get("message"))
            .or_else(|| value.get("error_description"))
            .or(error.filter(|e| e.is_string()))
            .and_then(serde_json::Value::as_str);
        if let Some(message) = message {
            return message.trim().to_owned();
        }
    }
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    match line.char_indices().nth(SUMMARY_CHARS) {
        Some((cut, _)) => format!("{}...", &line[..cut]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::GoogleError;

    fn display(body: &str) -> String {
        GoogleError::Http {
            status: 503,
            body: body.to_owned(),
        }
        .to_string()
    }

    #[test]
    fn shows_the_api_error_message() {
        let body = r#"{"error": {"code": 503, "message": "Backend Error",
            "errors": [{"reason": "backendError", "message": "Backend Error"}]}}"#;
        assert_eq!(display(body), "HTTP 503: Backend Error");
    }

    #[test]
    fn shows_the_token_endpoint_description_or_code() {
        let body = r#"{"error": "invalid_client", "error_description": "The OAuth client was not found."}"#;
        assert_eq!(display(body), "HTTP 503: The OAuth client was not found.");
        assert_eq!(
            display(r#"{"error": "invalid_client"}"#),
            "HTTP 503: invalid_client"
        );
    }

    #[test]
    fn shows_the_first_line_of_other_bodies() {
        let body = "\n<html><title>Service Unavailable</title>\n<body>secret details</body>";
        assert_eq!(
            display(body),
            "HTTP 503: <html><title>Service Unavailable</title>"
        );
    }

    #[test]
    fn truncates_long_lines() {
        let body = "é".repeat(500);
        let shown = display(&body);
        assert_eq!(shown, format!("HTTP 503: {}...", "é".repeat(120)));
    }

    #[test]
    fn json_without_a_message_falls_back_to_its_text() {
        assert_eq!(display(r#"{"oops": 1}"#), r#"HTTP 503: {"oops": 1}"#);
        assert_eq!(display(""), "HTTP 503: ");
    }

    #[test]
    fn keeps_the_whole_body_in_the_variant() {
        let body = "line one\nline two";
        let GoogleError::Http { body: kept, .. } = GoogleError::http(500, body.to_owned()) else {
            unreachable!()
        };
        assert_eq!(kept, body);
    }
}
