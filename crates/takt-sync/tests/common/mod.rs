#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use takt_google::{Auth, Endpoints, GoogleClient, OAuthClient, RetryPolicy};
use takt_store::Store;
use takt_sync::{AccountEngine, SyncEvent};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub fn oauth() -> OAuthClient {
    OAuthClient {
        client_id: "cid".into(),
        client_secret: "secret".into(),
    }
}

pub fn ok(body: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

/// One attempt per request so failures surface immediately.
pub fn no_retry() -> RetryPolicy {
    RetryPolicy {
        max_attempts: 1,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(1),
    }
}

pub fn client_at(base: &str, auth: Option<Auth>) -> GoogleClient {
    let endpoints = Endpoints::rooted_at(base);
    let http = reqwest::Client::new();
    let auth = auth.unwrap_or_else(|| {
        Auth::new(http.clone(), endpoints.token.clone(), oauth(), "rt")
            .with_access_token("t", Duration::from_secs(3600))
    });
    GoogleClient::new(http, endpoints, auth).with_retry(no_retry())
}

pub struct Fake {
    pub server: MockServer,
    pub store: Arc<Mutex<Store>>,
    pub events: async_channel::Receiver<SyncEvent>,
    pub engine: AccountEngine,
}

pub async fn fake() -> Fake {
    let server = MockServer::start().await;
    let engine_parts = engine_for(&server.uri(), None);
    Fake {
        server,
        store: engine_parts.0,
        events: engine_parts.1,
        engine: engine_parts.2,
    }
}

pub fn engine_for(
    base: &str,
    auth: Option<Auth>,
) -> (
    Arc<Mutex<Store>>,
    async_channel::Receiver<SyncEvent>,
    AccountEngine,
) {
    let mut store = Store::open_in_memory().unwrap();
    store.upsert_account("acc", "me@example.com").unwrap();
    let store = Arc::new(Mutex::new(store));
    let (tx, rx) = async_channel::unbounded();
    let engine = AccountEngine::new("acc", client_at(base, auth), store.clone(), tx);
    (store, rx, engine)
}

pub async fn mount_meta(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/calendar/v3/colors"))
        .respond_with(ok(json!({"calendar": {}, "event": {}})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/users/me/settings"))
        .respond_with(ok(
            json!({"items": [{"id": "format24HourTime", "value": "true"}]}),
        ))
        .mount(server)
        .await;
}

pub async fn mount_no_tasks(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/tasks/v1/users/@me/lists"))
        .respond_with(ok(json!({"items": []})))
        .mount(server)
        .await;
}

pub async fn mount_calendar_list(server: &MockServer) {
    Mock::given(method("GET")).and(path("/calendar/v3/users/me/calendarList"))
        .respond_with(ok(json!({
            "items": [{"id": "cal", "summary": "Me", "primary": true, "selected": true, "timeZone": "UTC"}],
            "nextSyncToken": "cl1"
        })))
        .mount(server).await;
}

pub fn event(id: &str, day: u32) -> Value {
    json!({
        "id": id, "summary": id,
        "start": {"dateTime": format!("2026-10-{day:02}T09:00:00Z")},
        "end": {"dateTime": format!("2026-10-{day:02}T10:00:00Z")}
    })
}

pub fn ids(store: &Arc<Mutex<Store>>) -> Vec<String> {
    store.lock().unwrap().event_ids("acc", "cal").unwrap()
}
