//! Google OAuth for installed apps: PKCE, a loopback redirect, and refresh.

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use reqwest::Url;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::{Endpoints, GoogleError};

pub const SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/calendar",
    "https://www.googleapis.com/auth/tasks",
    "openid",
    "email",
];

/// The scopes sign-in fails without; the user can untick them on the consent
/// screen. The identity scopes are not checked: the ID token proves them.
const REQUIRED_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/calendar",
    "https://www.googleapis.com/auth/tasks",
];

const REFRESH_MARGIN: Duration = Duration::from_secs(60);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthClient")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .finish()
    }
}

impl fmt::Debug for Pkce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pkce")
            .field("verifier", &"<redacted>")
            .field("challenge", &self.challenge)
            .finish()
    }
}

impl Pkce {
    pub fn generate() -> Self {
        Self::from_verifier(URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()))
    }

    pub fn from_verifier(verifier: impl Into<String>) -> Self {
        let verifier = verifier.into();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier,
            challenge,
        }
    }
}

pub fn authorization_url(
    endpoints: &Endpoints,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> Url {
    let mut url = endpoints.auth.clone();
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &SCOPES.join(" "))
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("access_type", "offline")
        // `consent` makes Google return a refresh token even on a repeat sign-in.
        .append_pair("prompt", "select_account consent");
    url
}

#[derive(Clone, PartialEq, Eq)]
pub enum Redirect {
    Code { code: String, state: String },
    Denied(String),
    Other,
}

impl fmt::Debug for Redirect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Redirect::Code { state, .. } => f
                .debug_struct("Code")
                .field("code", &"<redacted>")
                .field("state", state)
                .finish(),
            Redirect::Denied(reason) => f.debug_tuple("Denied").field(reason).finish(),
            Redirect::Other => f.write_str("Other"),
        }
    }
}

/// Reads the request line of the browser's redirect, e.g. `GET /?code=..&state=.. HTTP/1.1`.
pub fn parse_redirect_request_line(line: &str) -> Redirect {
    let Some(target) = line.split_whitespace().nth(1) else {
        return Redirect::Other;
    };
    let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Redirect::Other;
    };
    let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
    if let Some(error) = query.get("error") {
        return Redirect::Denied(error.clone());
    }
    match (query.get("code"), query.get("state")) {
        (Some(code), Some(state)) => Redirect::Code {
            code: code.clone(),
            state: state.clone(),
        },
        _ => Redirect::Other,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct IdClaims {
    pub sub: String,
    pub email: String,
}

/// Reads the claims of an ID token without checking its signature. OpenID
/// Connect allows this for a token received directly from the token endpoint
/// over TLS, which is the only place takt gets one.
pub fn decode_id_token(jwt: &str) -> Result<IdClaims, GoogleError> {
    let payload = jwt
        .split('.')
        .nth(1)
        .ok_or_else(|| GoogleError::AuthFlow("malformed id_token".into()))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .map_err(|e| GoogleError::AuthFlow(format!("id_token payload: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| GoogleError::AuthFlow(format!("id_token claims: {e}")))
}

#[derive(Clone)]
pub struct SignedIn {
    pub sub: String,
    pub email: String,
    pub refresh_token: String,
    pub access_token: String,
    pub expires_in: u64,
}

impl fmt::Debug for SignedIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignedIn")
            .field("sub", &self.sub)
            .field("email", &self.email)
            .field("refresh_token", &"<redacted>")
            .field("access_token", &"<redacted>")
            .field("expires_in", &self.expires_in)
            .finish()
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    id_token: Option<String>,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct TokenErrorBody {
    error: String,
}

/// Runs the whole browser sign-in: loopback listener, consent page, code exchange.
pub async fn sign_in(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    client: &OAuthClient,
    open_browser: impl FnOnce(&Url),
) -> Result<SignedIn, GoogleError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| GoogleError::AuthFlow(format!("cannot open loopback port: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| GoogleError::AuthFlow(e.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");
    let pkce = Pkce::generate();
    let state = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 16]>());
    open_browser(&authorization_url(
        endpoints,
        &client.client_id,
        &redirect_uri,
        &pkce.challenge,
        &state,
    ));
    let code = tokio::time::timeout(SIGN_IN_TIMEOUT, receive_code(&listener, &state))
        .await
        .map_err(|_| GoogleError::AuthFlow("timed out waiting for the browser".into()))??;
    exchange_code(
        http,
        endpoints,
        client,
        &code,
        &pkce.verifier,
        &redirect_uri,
    )
    .await
}

/// How long one browser connection may take to send its request line.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

async fn receive_code(listener: &TcpListener, expected_state: &str) -> Result<String, GoogleError> {
    loop {
        // A flaky or idle connection (browser preconnects, port scanners) must
        // not end the sign-in; only the redirect itself can.
        let Ok((mut stream, _)) = listener.accept().await else {
            // Accept errors (such as running out of file descriptors) repeat
            // at once; pause so the loop does not spin.
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        let mut buf = vec![0u8; 8192];
        let n = match tokio::time::timeout(READ_TIMEOUT, stream.read(&mut buf)).await {
            Ok(Ok(n)) => n,
            Ok(Err(_)) | Err(_) => continue,
        };
        let request = String::from_utf8_lossy(&buf[..n]);
        let line = request.lines().next().unwrap_or_default();
        match parse_redirect_request_line(line) {
            Redirect::Code { code, state } if state == expected_state => {
                respond(
                    &mut stream,
                    "200 OK",
                    "Signed in. You can close this tab and return to takt.",
                )
                .await;
                return Ok(code);
            }
            Redirect::Code { .. } => {
                respond(
                    &mut stream,
                    "400 Bad Request",
                    "Sign-in state did not match. Try again from takt.",
                )
                .await;
                return Err(GoogleError::AuthFlow("state mismatch".into()));
            }
            // An error redirect is only believed when it carries our state;
            // anything else could be a stray local request.
            Redirect::Denied(reason) if request_state(line).as_deref() == Some(expected_state) => {
                respond(
                    &mut stream,
                    "200 OK",
                    "Sign-in was cancelled. You can close this tab.",
                )
                .await;
                return Err(GoogleError::AuthFlow(format!("consent denied: {reason}")));
            }
            // Browsers also ask for /favicon.ico; ignore anything that is not the redirect.
            Redirect::Denied(_) | Redirect::Other => {
                respond(&mut stream, "404 Not Found", "").await
            }
        }
    }
}

fn request_state(line: &str) -> Option<String> {
    let target = line.split_whitespace().nth(1)?;
    let url = Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    url.query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
}

async fn respond(stream: &mut TcpStream, status: &str, message: &str) {
    let body = format!("<!doctype html><meta charset=utf-8><title>takt</title><p>{message}</p>");
    let reply = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(reply.as_bytes()).await;
}

pub async fn exchange_code(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    client: &OAuthClient,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<SignedIn, GoogleError> {
    let body = post_token_form(
        http,
        &endpoints.token,
        &[
            ("code", code),
            ("client_id", &client.client_id),
            ("client_secret", &client.client_secret),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier),
        ],
    )
    .await?;
    let token: TokenResponse =
        serde_json::from_str(&body).map_err(|e| GoogleError::Decode(e.to_string()))?;
    if let Some(granted) = &token.scope {
        let granted: Vec<&str> = granted.split_whitespace().collect();
        let missing: Vec<&str> = REQUIRED_SCOPES
            .iter()
            .copied()
            .filter(|required| !granted.contains(required))
            .collect();
        if !missing.is_empty() {
            return Err(GoogleError::AuthFlow(format!(
                "Google did not grant the access takt needs (missing {}); sign in again and allow Calendar and Tasks access",
                missing.join(", ")
            )));
        }
    }
    let id_token = token
        .id_token
        .ok_or_else(|| GoogleError::AuthFlow("Google returned no id_token".into()))?;
    let claims = decode_id_token(&id_token)?;
    let refresh_token = token.refresh_token.ok_or_else(|| {
        GoogleError::AuthFlow(
            "Google returned no refresh token; remove takt under Third-party access in your Google Account and sign in again".into(),
        )
    })?;
    Ok(SignedIn {
        sub: claims.sub,
        email: claims.email,
        refresh_token,
        access_token: token.access_token,
        expires_in: token.expires_in,
    })
}

async fn post_token_form(
    http: &reqwest::Client,
    url: &Url,
    form: &[(&str, &str)],
) -> Result<String, GoogleError> {
    let response = http
        .post(url.clone())
        .form(form)
        .send()
        .await
        .map_err(GoogleError::from_reqwest)?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(GoogleError::from_reqwest)?;
    if (200..300).contains(&status) {
        Ok(body)
    } else {
        Err(token_error(status, &body))
    }
}

fn token_error(status: u16, body: &str) -> GoogleError {
    match serde_json::from_str::<TokenErrorBody>(body) {
        Ok(e) if e.error == "invalid_grant" => GoogleError::InvalidGrant,
        _ => GoogleError::http(status, body.to_owned()),
    }
}

#[derive(Deserialize)]
struct RefreshResponse {
    access_token: String,
    expires_in: u64,
}

/// One account's credentials. Hands out access tokens, refreshing as needed.
pub struct Auth {
    http: reqwest::Client,
    token_url: Url,
    client: OAuthClient,
    refresh_token: String,
    cache: Mutex<Option<(String, Instant)>>,
}

impl fmt::Debug for Auth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Auth")
            .field("client", &self.client)
            .field("refresh_token", &"<redacted>")
            .field("cache", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl Auth {
    pub fn new(
        http: reqwest::Client,
        token_url: Url,
        client: OAuthClient,
        refresh_token: impl Into<String>,
    ) -> Self {
        Self {
            http,
            token_url,
            client,
            refresh_token: refresh_token.into(),
            cache: Mutex::new(None),
        }
    }

    /// Seeds the cache, e.g. with the access token a sign-in just returned.
    pub fn with_access_token(mut self, token: impl Into<String>, expires_in: Duration) -> Self {
        *self.cache.get_mut() = Some((token.into(), Instant::now() + expires_in));
        self
    }

    pub async fn access_token(&self, force_refresh: bool) -> Result<String, GoogleError> {
        let mut cache = self.cache.lock().await;
        if !force_refresh
            && let Some((token, expires_at)) = cache.as_ref()
            && Instant::now() + REFRESH_MARGIN < *expires_at
        {
            return Ok(token.clone());
        }
        self.refresh(&mut cache).await
    }

    /// Refreshes after the API rejected `rejected`, unless another caller
    /// already replaced that token, so concurrent 401s cause one refresh.
    pub async fn refresh_if_stale(&self, rejected: &str) -> Result<String, GoogleError> {
        let mut cache = self.cache.lock().await;
        if let Some((token, _)) = cache.as_ref()
            && token != rejected
        {
            return Ok(token.clone());
        }
        self.refresh(&mut cache).await
    }

    async fn refresh(&self, cache: &mut Option<(String, Instant)>) -> Result<String, GoogleError> {
        let body = post_token_form(
            &self.http,
            &self.token_url,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", &self.refresh_token),
                ("client_id", &self.client.client_id),
                ("client_secret", &self.client.client_secret),
            ],
        )
        .await?;
        let fresh: RefreshResponse =
            serde_json::from_str(&body).map_err(|e| GoogleError::Decode(e.to_string()))?;
        *cache = Some((
            fresh.access_token.clone(),
            Instant::now() + Duration::from_secs(fresh.expires_in),
        ));
        Ok(fresh.access_token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_scopes_are_requested() {
        assert!(REQUIRED_SCOPES.iter().all(|s| SCOPES.contains(s)));
    }
}
