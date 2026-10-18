use std::collections::HashMap;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::json;
use takt_google::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn oauth() -> OAuthClient {
    OAuthClient {
        client_id: "cid".into(),
        client_secret: "secret".into(),
    }
}

fn id_token(sub: &str, email: &str) -> String {
    let payload = URL_SAFE_NO_PAD.encode(json!({"sub": sub, "email": email}).to_string());
    format!("eyJhbGciOiJSUzI1NiJ9.{payload}.signature")
}

#[test]
fn pkce_matches_rfc7636_example() {
    let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
    assert_eq!(
        pkce.challenge,
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn generated_verifier_is_43_chars() {
    assert_eq!(Pkce::generate().verifier.len(), 43);
}

#[test]
fn authorization_url_carries_pkce_scopes_and_offline_access() {
    let url = authorization_url(
        &Endpoints::google(),
        "cid",
        "http://127.0.0.1:5555",
        "chal",
        "st",
    );
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q["code_challenge"], "chal");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["access_type"], "offline");
    assert_eq!(q["scope"], SCOPES.join(" "));
    assert_eq!(q["redirect_uri"], "http://127.0.0.1:5555");
    assert_eq!(q["state"], "st");
    assert_eq!(q["response_type"], "code");
}

#[test]
fn redirect_request_lines() {
    assert_eq!(
        parse_redirect_request_line("GET /?state=s1&code=4%2Fabc HTTP/1.1"),
        Redirect::Code {
            code: "4/abc".into(),
            state: "s1".into()
        }
    );
    assert_eq!(
        parse_redirect_request_line("GET /?error=access_denied&state=s1 HTTP/1.1"),
        Redirect::Denied("access_denied".into())
    );
    assert_eq!(
        parse_redirect_request_line("GET /favicon.ico HTTP/1.1"),
        Redirect::Other
    );
    assert_eq!(parse_redirect_request_line(""), Redirect::Other);
}

#[test]
fn id_token_claims_decode() {
    let claims = decode_id_token(&id_token("123", "a@example.com")).unwrap();
    assert_eq!(
        claims,
        IdClaims {
            sub: "123".into(),
            email: "a@example.com".into()
        }
    );
    assert!(decode_id_token("not-a-jwt").is_err());
}

async fn mount_code_exchange(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=the-code"))
        .and(body_string_contains("code_verifier="))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "at", "expires_in": 3599, "refresh_token": "rt",
            "id_token": id_token("sub-1", "me@example.com")
        })))
        .mount(server)
        .await;
}

fn browser_that_returns(state_override: Option<&'static str>) -> impl FnOnce(&Url) {
    move |auth_url: &Url| {
        let q: HashMap<String, String> = auth_url.query_pairs().into_owned().collect();
        let state = state_override
            .map(str::to_owned)
            .unwrap_or_else(|| q["state"].clone());
        let callback = format!("{}/?code=the-code&state={state}", q["redirect_uri"]);
        tokio::spawn(async move {
            let _ = reqwest::get(callback).await;
        });
    }
}

#[tokio::test]
async fn sign_in_round_trip() {
    let server = MockServer::start().await;
    mount_code_exchange(&server).await;
    let endpoints = Endpoints::rooted_at(&server.uri());
    let signed = sign_in(
        &reqwest::Client::new(),
        &endpoints,
        &oauth(),
        browser_that_returns(None),
    )
    .await
    .unwrap();
    assert_eq!(signed.sub, "sub-1");
    assert_eq!(signed.email, "me@example.com");
    assert_eq!(signed.refresh_token, "rt");
    assert_eq!(signed.access_token, "at");
}

#[tokio::test]
async fn sign_in_rejects_state_mismatch() {
    let server = MockServer::start().await;
    mount_code_exchange(&server).await;
    let endpoints = Endpoints::rooted_at(&server.uri());
    let err = sign_in(
        &reqwest::Client::new(),
        &endpoints,
        &oauth(),
        browser_that_returns(Some("forged")),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, GoogleError::AuthFlow(_)));
}

async fn mount_refresh(server: &MockServer, token: &str, expect: u64) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=rt"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": token, "expires_in": 3600})),
        )
        .expect(expect)
        .mount(server)
        .await;
}

fn auth_for(server: &MockServer) -> Auth {
    let endpoints = Endpoints::rooted_at(&server.uri());
    Auth::new(reqwest::Client::new(), endpoints.token, oauth(), "rt")
}

#[tokio::test]
async fn cached_access_token_is_reused() {
    let server = MockServer::start().await;
    mount_refresh(&server, "at1", 1).await;
    let auth = auth_for(&server);
    assert_eq!(auth.access_token(false).await.unwrap(), "at1");
    assert_eq!(auth.access_token(false).await.unwrap(), "at1");
}

#[tokio::test]
async fn force_refresh_hits_token_endpoint() {
    let server = MockServer::start().await;
    mount_refresh(&server, "new", 1).await;
    let auth = auth_for(&server).with_access_token("old", Duration::from_secs(3600));
    assert_eq!(auth.access_token(true).await.unwrap(), "new");
}

#[tokio::test]
async fn token_close_to_expiry_is_refreshed() {
    let server = MockServer::start().await;
    mount_refresh(&server, "new", 1).await;
    let auth = auth_for(&server).with_access_token("old", Duration::from_secs(30));
    assert_eq!(auth.access_token(false).await.unwrap(), "new");
}

#[tokio::test]
async fn invalid_grant_maps_to_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "error": "invalid_grant", "error_description": "Token has been expired or revoked."
        })))
        .mount(&server)
        .await;
    let err = auth_for(&server).access_token(false).await.unwrap_err();
    assert!(matches!(err, GoogleError::InvalidGrant));
}

#[test]
fn memory_token_store_round_trip() {
    let store = MemoryTokenStore::default();
    assert_eq!(store.get("a@example.com").unwrap(), None);
    store.set("a@example.com", "rt").unwrap();
    assert_eq!(store.get("a@example.com").unwrap().as_deref(), Some("rt"));
    store.delete("a@example.com").unwrap();
    assert_eq!(store.get("a@example.com").unwrap(), None);
}

#[test]
#[ignore = "touches the real OS keychain; run with --ignored"]
fn keyring_token_store_round_trip() {
    let store = KeyringTokenStore;
    let email = "takt-test@example.invalid";
    store.set(email, "rt-test").unwrap();
    assert_eq!(store.get(email).unwrap().as_deref(), Some("rt-test"));
    store.delete(email).unwrap();
    assert_eq!(store.get(email).unwrap(), None);
}
