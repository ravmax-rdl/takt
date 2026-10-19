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
        .expect(2)
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
    let err = client_at(&server.uri())
        .get_json(colors_url(&server))
        .await
        .unwrap_err();
    assert!(matches!(err, GoogleError::Http { status: 401, .. }));
}

fn slow_backoff_client(base: &str) -> GoogleClient {
    client_at(base).with_retry(RetryPolicy {
        max_attempts: 5,
        base_delay: Duration::from_secs(30),
        max_delay: Duration::from_secs(60),
    })
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
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        slow_backoff_client(&server.uri()).get_json(colors_url(&server)),
    )
    .await
    .expect("Retry-After: 0 must override the 30 s backoff");
    assert!(result.is_ok());
}

#[tokio::test]
async fn retry_after_http_date_in_the_past_waits_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header("Retry-After", "Wed, 21 Oct 2015 07:28:00 GMT"),
        )
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        slow_backoff_client(&server.uri()).get_json(colors_url(&server)),
    )
    .await
    .expect("an HTTP-date Retry-After must be parsed");
    assert!(result.is_ok());
}

#[tokio::test]
async fn huge_retry_after_is_clamped_to_max_delay() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "86400"))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    // fast_retry caps the wait at 4 ms.
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        client_at(&server.uri()).get_json(colors_url(&server)),
    )
    .await
    .expect("Retry-After must be bounded by max_delay");
    assert!(result.is_ok());
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
async fn user_rate_limit_403_retries_then_reports() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(error_body(403, "userRateLimitExceeded")),
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
async fn rate_limit_reason_is_found_past_the_first_entry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error": {
            "code": 403,
            "errors": [{"reason": "other"}, {"reason": "rateLimitExceeded"}]
        }})))
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
async fn rate_limit_exceeded_detail_reason_retries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error": {
            "code": 403,
            "details": [{"@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "RATE_LIMIT_EXCEEDED"}]
        }})))
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
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/missing"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
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
    // Bind to get a free port, then close it so nothing listens there.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let base = format!("http://127.0.0.1:{port}");
    let err = client_at(&base)
        .get_json(Url::parse(&format!("{base}/calendar/v3/colors?syncToken=secret")).unwrap())
        .await
        .unwrap_err();
    let GoogleError::Network(message) = err else {
        panic!("expected a network error, got {err:?}");
    };
    assert!(!message.contains("secret"), "URL leaked: {message}");
    assert!(message.len() > "error sending request".len(), "{message}");
}
