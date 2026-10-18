//! Google Calendar v3 and Tasks v1 clients for takt.

mod auth;
mod client;
mod error;
pub mod model;
mod token_store;

pub use auth::{
    Auth, IdClaims, OAuthClient, Pkce, Redirect, SCOPES, SignedIn, authorization_url,
    decode_id_token, exchange_code, parse_redirect_request_line, sign_in,
};
pub use client::{GoogleClient, RetryPolicy};
pub use error::GoogleError;
pub use reqwest::Url;
pub use token_store::{KeyringTokenStore, MemoryTokenStore, TokenStore};

/// Base URLs for Google's OAuth, Calendar and Tasks APIs.
#[derive(Debug, Clone)]
pub struct Endpoints {
    pub auth: Url,
    pub token: Url,
    pub calendar: Url,
    pub tasks: Url,
}

impl Endpoints {
    pub fn google() -> Self {
        Self {
            auth: url("https://accounts.google.com/o/oauth2/v2/auth"),
            token: url("https://oauth2.googleapis.com/token"),
            calendar: url("https://www.googleapis.com/calendar/v3/"),
            tasks: url("https://tasks.googleapis.com/tasks/v1/"),
        }
    }

    /// Endpoints under one test server, e.g. a wiremock `MockServer::uri()`.
    pub fn rooted_at(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self {
            auth: url(&format!("{base}/auth")),
            token: url(&format!("{base}/token")),
            calendar: url(&format!("{base}/calendar/v3/")),
            tasks: url(&format!("{base}/tasks/v1/")),
        }
    }
}

fn url(s: &str) -> Url {
    Url::parse(s).expect("static URL is valid")
}
