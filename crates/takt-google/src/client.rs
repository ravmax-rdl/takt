//! Authenticated GET with Google's retry rules.

use std::time::Duration;

use chrono::{DateTime, Utc};
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
            let retry_after =
                retry_after(response.headers()).map(|wait| wait.min(self.retry.max_delay));
            let body = response.text().await.map_err(GoogleError::from_reqwest)?;
            if status.is_success() {
                return serde_json::from_str(&body).map_err(|e| GoogleError::Decode(e.to_string()));
            }
            match classify(status, &body) {
                Outcome::RefreshAndRetry if !refreshed => {
                    refreshed = true;
                    attempt -= 1;
                    self.auth.refresh_if_stale(&token).await?;
                }
                Outcome::RefreshAndRetry => {
                    return Err(GoogleError::http(status.as_u16(), body));
                }
                Outcome::Retry if attempt < self.retry.max_attempts => {
                    let wait = retry_after.unwrap_or_else(|| self.retry.delay(attempt));
                    tracing::debug!(%status, ?wait, attempt, "retrying Google request");
                    tokio::time::sleep(wait).await;
                }
                Outcome::Retry if status.is_server_error() => {
                    return Err(GoogleError::http(status.as_u16(), body));
                }
                Outcome::Retry => return Err(GoogleError::RateLimited),
                Outcome::Fail(error) => return Err(error),
            }
        }
    }
}

/// `Retry-After` is either delay-seconds or an HTTP-date (RFC 9110 5.6.7).
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let date = DateTime::parse_from_rfc2822(value).ok()?;
    Some(
        (date.with_timezone(&Utc) - Utc::now())
            .to_std()
            .unwrap_or_default(),
    )
}

const RATE_LIMIT_REASONS: &[&str] = &["rateLimitExceeded", "userRateLimitExceeded"];

/// The reason that decides how a 403 is handled: a rate-limit reason anywhere
/// in the body wins, otherwise the first reason Google gave.
fn error_reason(body: &str) -> Option<String> {
    let value: Value = serde_json::from_str(body).ok()?;
    let reasons = |pointer: &str| -> Vec<String> {
        value
            .pointer(pointer)
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|e| e.get("reason")?.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let errors = reasons("/error/errors");
    let details = reasons("/error/details");
    if let Some(reason) = errors
        .iter()
        .find(|r| RATE_LIMIT_REASONS.contains(&r.as_str()))
    {
        return Some(reason.clone());
    }
    if details.iter().any(|r| r == "RATE_LIMIT_EXCEEDED") {
        return Some("rateLimitExceeded".to_owned());
    }
    errors
        .into_iter()
        .next()
        .or_else(|| details.into_iter().next())
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
        code => Outcome::Fail(GoogleError::http(code, body.to_owned())),
    }
}
