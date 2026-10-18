use std::time::Duration;

use serde_json::json;
use takt_google::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn oauth() -> OAuthClient {
    OAuthClient {
        client_id: "cid".into(),
        client_secret: "secret".into(),
    }
}

fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 5,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(4),
    }
}

fn client_at(base: &str) -> GoogleClient {
    let endpoints = Endpoints::rooted_at(base);
    let http = reqwest::Client::new();
    let auth = Auth::new(http.clone(), endpoints.token.clone(), oauth(), "rt")
        .with_access_token("t1", Duration::from_secs(3600));
    GoogleClient::new(http, endpoints, auth).with_retry(fast_retry())
}

fn colors_url(server: &MockServer) -> Url {
    Url::parse(&format!("{}/calendar/v3/colors", server.uri())).unwrap()
}

#[tokio::test]
async fn sends_bearer_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/colors"))
        .and(header("authorization", "Bearer t1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let body = client_at(&server.uri())
        .get_json(colors_url(&server))
        .await
        .unwrap();
    assert_eq!(body, json!({"ok": true}));
}

#[tokio::test]
async fn refreshes_once_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer t1"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": "t2", "expires_in": 3600})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(header("authorization", "Bearer t2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        client_at(&server.uri())
            .get_json(colors_url(&server))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn second_401_is_an_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": "t2", "expires_in": 3600})),
        )
        .mount(&server)
        .await;
    let err = client_at(&server.uri())
        .get_json(colors_url(&server))
        .await
        .unwrap_err();
    assert!(matches!(err, GoogleError::Http { status: 401, .. }));
}

#[tokio::test]
async fn honours_retry_after_on_429() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        client_at(&server.uri())
            .get_json(colors_url(&server))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn server_errors_retry_then_fail() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .expect(5)
        .mount(&server)
        .await;
    let err = client_at(&server.uri())
        .get_json(colors_url(&server))
        .await
        .unwrap_err();
    assert!(matches!(err, GoogleError::Http { status: 503, .. }));
}

fn error_body(code: u16, reason: &str) -> serde_json::Value {
    json!({"error": {"code": code, "message": reason, "errors": [{"reason": reason, "domain": "usageLimits"}]}})
}

#[tokio::test]
async fn rate_limit_403_retries_then_reports() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(error_body(403, "rateLimitExceeded")),
        )
        .expect(5)
        .mount(&server)
        .await;
    let err = client_at(&server.uri())
        .get_json(colors_url(&server))
        .await
        .unwrap_err();
    assert!(matches!(err, GoogleError::RateLimited));
}

#[tokio::test]
async fn plain_403_is_forbidden_without_retry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_json(error_body(403, "forbidden")))
        .expect(1)
        .mount(&server)
        .await;
    let err = client_at(&server.uri())
        .get_json(colors_url(&server))
        .await
        .unwrap_err();
    assert!(matches!(err, GoogleError::Forbidden { ref reason } if reason == "forbidden"));
}

#[tokio::test]
async fn gone_and_not_found_are_typed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/gone"))
        .respond_with(ResponseTemplate::new(410))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let client = client_at(&server.uri());
    let gone = Url::parse(&format!("{}/calendar/v3/gone", server.uri())).unwrap();
    let missing = Url::parse(&format!("{}/calendar/v3/missing", server.uri())).unwrap();
    assert!(matches!(
        client.get_json(gone).await,
        Err(GoogleError::Gone)
    ));
    assert!(matches!(
        client.get_json(missing).await,
        Err(GoogleError::NotFound)
    ));
}

#[tokio::test]
async fn connection_refused_is_a_network_error() {
    // Nothing listens on the discard port locally.
    let client = client_at("http://127.0.0.1:9");
    let err = client
        .get_json(Url::parse("http://127.0.0.1:9/calendar/v3/colors").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, GoogleError::Network(_)));
}
