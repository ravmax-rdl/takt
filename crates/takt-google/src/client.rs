//! Authenticated GET with Google's retry rules.

use std::time::Duration;

use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::{StatusCode, Url};
use serde_json::Value;

use crate::{Auth, Endpoints, GoogleError};

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(32),
        }
    }
}

impl RetryPolicy {
    fn delay(&self, attempt: u32) -> Duration {
        let exponential = self
            .base_delay
            .saturating_mul(1u32 << attempt.saturating_sub(1).min(16));
        let jitter = rand::random_range(0..=self.base_delay.as_millis() as u64);
        exponential.min(self.max_delay) + Duration::from_millis(jitter)
    }
}

pub struct GoogleClient {
    pub(crate) http: reqwest::Client,
    pub(crate) endpoints: Endpoints,
    auth: Auth,
    retry: RetryPolicy,
}

enum Outcome {
    Retry,
    RefreshAndRetry,
    Fail(GoogleError),
}

impl GoogleClient {
    pub fn new(http: reqwest::Client, endpoints: Endpoints, auth: Auth) -> Self {
        Self {
            http,
            endpoints,
            auth,
            retry: RetryPolicy::default(),
        }
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    pub async fn get_json(&self, url: Url) -> Result<Value, GoogleError> {
        let mut attempt = 0;
        let mut refreshed = false;
        loop {
            attempt += 1;
            let token = self.auth.access_token(false).await?;
            let response = self
                .http
                .get(url.clone())
                .bearer_auth(&token)
                .send()
                .await
                .map_err(GoogleError::from_reqwest)?;
            let status = response.status();
            let retry_after = retry_after(response.headers());
            let body = response.text().await.map_err(GoogleError::from_reqwest)?;
            if status.is_success() {
                return serde_json::from_str(&body).map_err(|e| GoogleError::Decode(e.to_string()));
            }
            match classify(status, &body) {
                Outcome::RefreshAndRetry if !refreshed => {
                    refreshed = true;
                    attempt -= 1;
                    self.auth.access_token(true).await?;
                }
                Outcome::RefreshAndRetry => {
                    return Err(GoogleError::Http {
                        status: status.as_u16(),
                        body,
                    });
                }
                Outcome::Retry if attempt < self.retry.max_attempts => {
                    let wait = retry_after.unwrap_or_else(|| self.retry.delay(attempt));
                    tracing::debug!(%status, ?wait, attempt, "retrying Google request");
                    tokio::time::sleep(wait).await;
                }
                Outcome::Retry if status.is_server_error() => {
                    return Err(GoogleError::Http {
                        status: status.as_u16(),
                        body,
                    });
                }
                Outcome::Retry => return Err(GoogleError::RateLimited),
                Outcome::Fail(error) => return Err(error),
            }
        }
    }
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let seconds = headers
        .get(RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(Duration::from_secs(seconds))
}

fn error_reason(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    value
        .pointer("/error/errors/0/reason")?
        .as_str()
        .map(str::to_owned)
}

fn classify(status: StatusCode, body: &str) -> Outcome {
    match status.as_u16() {
        401 => Outcome::RefreshAndRetry,
        404 => Outcome::Fail(GoogleError::NotFound),
        410 => Outcome::Fail(GoogleError::Gone),
        429 => Outcome::Retry,
        403 => match error_reason(body).as_deref() {
            Some("rateLimitExceeded" | "userRateLimitExceeded") => Outcome::Retry,
            reason => Outcome::Fail(GoogleError::Forbidden {
                reason: reason.unwrap_or("forbidden").to_owned(),
            }),
        },
        500..=599 => Outcome::Retry,
        code => Outcome::Fail(GoogleError::Http {
            status: code,
            body: body.to_owned(),
        }),
    }
}
