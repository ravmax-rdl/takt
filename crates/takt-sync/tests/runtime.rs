mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::time::Duration;

use base64::Engine;
use common::*;
use serde_json::json;
use takt_google::{Endpoints, GoogleError, MemoryTokenStore, RetryPolicy, TokenStore, Url};
use takt_store::{AccountStatus, Store};
use takt_sync::*;
use wiremock::matchers::{body_string_contains, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Rig {
    server: MockServer,
    dir: tempfile::TempDir,
    tokens: Arc<MemoryTokenStore>,
    /// How many of the next keychain reads fail.
    failing_reads: Arc<AtomicUsize>,
}

/// A keychain that can be made to fail reads, like a locked one.
struct FlakyTokens {
    inner: Arc<MemoryTokenStore>,
    failing_reads: Arc<AtomicUsize>,
}

impl TokenStore for FlakyTokens {
    fn get(&self, email: &str) -> Result<Option<String>, GoogleError> {
        let failing = self
            .failing_reads
            .try_update(SeqCst, SeqCst, |n| n.checked_sub(1))
            .is_ok();
        if failing {
            return Err(GoogleError::TokenStore("keychain locked".into()));
        }
        self.inner.get(email)
    }

    fn set(&self, email: &str, refresh_token: &str) -> Result<(), GoogleError> {
        self.inner.set(email, refresh_token)
    }

    fn delete(&self, email: &str) -> Result<(), GoogleError> {
        self.inner.delete(email)
    }
}

impl Rig {
    async fn new() -> Self {
        Self {
            server: MockServer::start().await,
            dir: tempfile::tempdir().unwrap(),
            tokens: Arc::new(MemoryTokenStore::default()),
            failing_reads: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn db(&self) -> std::path::PathBuf {
        self.dir.path().join("takt.sqlite3")
    }

    fn seed_account(&self) {
        Store::open(&self.db())
            .unwrap()
            .upsert_account("acc", "me@example.com")
            .unwrap();
        self.tokens.set("me@example.com", "rt").unwrap();
    }

    fn start(&self, open_browser: Arc<dyn Fn(&Url) + Send + Sync>) -> SyncHandle {
        self.start_at(&self.server.uri(), open_browser)
    }

    /// Like `start`, with Google at `base` instead of the rig's server.
    fn start_at(&self, base: &str, open_browser: Arc<dyn Fn(&Url) + Send + Sync>) -> SyncHandle {
        start(SyncConfig {
            db_path: self.db(),
            endpoints: Endpoints::rooted_at(base),
            oauth: oauth(),
            tokens: Arc::new(FlakyTokens {
                inner: self.tokens.clone(),
                failing_reads: self.failing_reads.clone(),
            }),
            open_browser,
            intervals: Intervals {
                focused: Duration::from_secs(3600),
                unfocused: Duration::from_secs(3600),
                ..Intervals::default()
            },
            retry: RetryPolicy {
                max_attempts: 1,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(1),
            },
        })
        .unwrap()
    }

    async fn mount_happy_google(&self) {
        mount_google_except_events(&self.server).await;
        Mock::given(method("GET"))
            .and(path("/calendar/v3/calendars/cal/events"))
            .respond_with(ok(json!({"items": [], "nextSyncToken": "s"})))
            .mount(&self.server)
            .await;
    }
}

/// Token refresh, calendar list, metadata and (no) tasks.
async fn mount_google_except_events(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .respond_with(ok(json!({"access_token": "at", "expires_in": 3600})))
        .mount(server)
        .await;
    mount_calendar_list(server).await;
    mount_meta(server).await;
    mount_no_tasks(server).await;
}

fn no_browser() -> Arc<dyn Fn(&Url) + Send + Sync> {
    Arc::new(|_| {})
}

async fn wait_for(
    events: &async_channel::Receiver<SyncEvent>,
    wanted: impl Fn(&SyncEvent) -> bool,
) -> SyncEvent {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let ev = events.recv().await.expect("sync thread ended");
            if wanted(&ev) {
                return ev;
            }
        }
    })
    .await
    .expect("expected event did not arrive")
}

fn synced(account: &str) -> impl Fn(&SyncEvent) -> bool + '_ {
    move |ev| matches!(ev, SyncEvent::Status { account_id, state: SyncState::Synced(_) } if account_id == account)
}

#[tokio::test]
async fn existing_account_syncs_on_start_and_on_sync_now() {
    let rig = Rig::new().await;
    rig.seed_account();
    rig.mount_happy_google().await;
    let handle = rig.start(no_browser());
    wait_for(&handle.events, synced("acc")).await;
    handle.commands.send(Command::SyncNow).await.unwrap();
    wait_for(&handle.events, synced("acc")).await;
    let list_calls = rig
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == "/calendar/v3/users/me/calendarList")
        .count();
    assert!(list_calls >= 2);
    handle.shutdown();
}

#[tokio::test]
async fn revoked_token_marks_account_and_asks_for_sign_in() {
    let rig = Rig::new().await;
    rig.seed_account();
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
        .mount(&rig.server)
        .await;
    let handle = rig.start(no_browser());
    wait_for(
        &handle.events,
        |ev| matches!(ev, SyncEvent::AuthNeeded { account_id } if account_id == "acc"),
    )
    .await;
    let accounts = Store::open_read_only(&rig.db())
        .unwrap()
        .accounts()
        .unwrap();
    assert_eq!(accounts[0].status, AccountStatus::NeedsReauth);
    handle.shutdown();
}

#[tokio::test]
async fn missing_refresh_token_asks_for_sign_in() {
    let rig = Rig::new().await;
    Store::open(&rig.db())
        .unwrap()
        .upsert_account("acc", "me@example.com")
        .unwrap();
    let handle = rig.start(no_browser());
    wait_for(&handle.events, |ev| {
        matches!(ev, SyncEvent::AuthNeeded { .. })
    })
    .await;
    handle.shutdown();
}

#[tokio::test]
async fn remove_account_deletes_rows_and_token() {
    let rig = Rig::new().await;
    rig.seed_account();
    rig.mount_happy_google().await;
    let handle = rig.start(no_browser());
    wait_for(&handle.events, synced("acc")).await;
    handle
        .commands
        .send(Command::RemoveAccount {
            account_id: "acc".into(),
        })
        .await
        .unwrap();
    wait_for(&handle.events, |ev| {
        matches!(ev, SyncEvent::AccountsChanged)
    })
    .await;
    assert!(
        Store::open_read_only(&rig.db())
            .unwrap()
            .accounts()
            .unwrap()
            .is_empty()
    );
    assert_eq!(rig.tokens.get("me@example.com").unwrap(), None);
    handle.shutdown();
}

#[tokio::test]
async fn add_account_signs_in_and_starts_syncing() {
    let rig = Rig::new().await;
    rig.mount_happy_google().await;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(json!({"sub": "sub-1", "email": "new@example.com"}).to_string());
    Mock::given(method("POST")).and(path("/token")).and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ok(json!({"access_token": "at", "expires_in": 3600, "refresh_token": "rt-new", "id_token": format!("h.{payload}.s")})))
        .mount(&rig.server).await;
    let browser: Arc<dyn Fn(&Url) + Send + Sync> = Arc::new(|url: &Url| {
        let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
        let callback = format!("{}/?code=c&state={}", q["redirect_uri"], q["state"]);
        tokio::spawn(async move {
            let _ = reqwest::get(callback).await;
        });
    });
    let handle = rig.start(browser);
    handle.commands.send(Command::AddAccount).await.unwrap();
    wait_for(&handle.events, |ev| {
        matches!(ev, SyncEvent::AccountsChanged)
    })
    .await;
    wait_for(&handle.events, synced("sub-1")).await;
    assert_eq!(
        rig.tokens.get("new@example.com").unwrap().as_deref(),
        Some("rt-new")
    );
    handle.shutdown();
}

fn calendar_list_calls(requests: &[wiremock::Request]) -> usize {
    requests
        .iter()
        .filter(|r| r.url.path() == "/calendar/v3/users/me/calendarList")
        .count()
}

#[tokio::test]
async fn keychain_read_error_is_retried() {
    let rig = Rig::new().await;
    rig.seed_account();
    rig.mount_happy_google().await;
    rig.failing_reads.store(1, SeqCst);
    let handle = rig.start(no_browser());
    wait_for(&handle.events, |ev| {
        matches!(
            ev,
            SyncEvent::Status {
                state: SyncState::Error(_),
                ..
            }
        )
    })
    .await;
    // A wake-up retries at once instead of waiting out the backoff.
    handle.commands.send(Command::SyncNow).await.unwrap();
    wait_for(&handle.events, synced("acc")).await;
    handle.shutdown();
}

#[tokio::test]
async fn new_account_ignores_earlier_wake_ups() {
    let rig = Rig::new().await;
    rig.mount_happy_google().await;
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(json!({"sub": "sub-1", "email": "new@example.com"}).to_string());
    Mock::given(method("POST")).and(path("/token")).and(body_string_contains("grant_type=authorization_code"))
        .respond_with(ok(json!({"access_token": "at", "expires_in": 3600, "refresh_token": "rt-new", "id_token": format!("h.{payload}.s")})))
        .mount(&rig.server).await;
    let browser: Arc<dyn Fn(&Url) + Send + Sync> = Arc::new(|url: &Url| {
        let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
        let callback = format!("{}/?code=c&state={}", q["redirect_uri"], q["state"]);
        tokio::spawn(async move {
            let _ = reqwest::get(callback).await;
        });
    });
    let handle = rig.start(browser);
    // Sent before the account exists; its loop must not take it as a wake-up.
    handle.commands.send(Command::SyncNow).await.unwrap();
    handle.commands.send(Command::AddAccount).await.unwrap();
    wait_for(&handle.events, synced("sub-1")).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let requests = rig.server.received_requests().await.unwrap();
    assert_eq!(calendar_list_calls(&requests), 1, "one round only");
    handle.shutdown();
}

fn stored_event_ids(rig: &Rig) -> Vec<String> {
    Store::open_read_only(&rig.db())
        .unwrap()
        .event_ids("acc", "cal")
        .unwrap()
}

#[tokio::test]
async fn offline_account_recovers_on_sync_now_and_keeps_its_rows() {
    let rig = Rig::new().await;
    rig.seed_account();
    // An earlier run stored one event and a sync token.
    mount_google_except_events(&rig.server).await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/cal/events"))
        .respond_with(ok(
            json!({"items": [event("e1", 20)], "nextSyncToken": "s1"}),
        ))
        .mount(&rig.server)
        .await;
    let handle = rig.start(no_browser());
    wait_for(&handle.events, synced("acc")).await;
    handle.shutdown();
    assert_eq!(stored_event_ids(&rig), ["e1"]);

    // Google is unreachable: nothing listens on the port, so connections are refused.
    let address = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let handle = rig.start_at(&format!("http://{address}"), no_browser());
    wait_for(&handle.events, |ev| {
        matches!(ev, SyncEvent::Status { account_id, state: SyncState::Offline(_) } if account_id == "acc")
    })
    .await;
    assert_eq!(stored_event_ids(&rig), ["e1"], "offline keeps stored rows");

    // Google comes back on the same address. Only an incremental sync from
    // the stored token is answered, so a full resync would fail the test.
    let listener = std::net::TcpListener::bind(address).unwrap();
    let google = MockServer::builder().listener(listener).start().await;
    mount_google_except_events(&google).await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/calendars/cal/events"))
        .and(query_param("syncToken", "s1"))
        .respond_with(ok(json!({"items": [], "nextSyncToken": "s2"})))
        .mount(&google)
        .await;
    handle.commands.send(Command::SyncNow).await.unwrap();
    wait_for(&handle.events, synced("acc")).await;
    assert_eq!(stored_event_ids(&rig), ["e1"]);
    assert_eq!(
        Store::open_read_only(&rig.db())
            .unwrap()
            .events_sync_token("acc", "cal")
            .unwrap()
            .as_deref(),
        Some("s2")
    );
    handle.shutdown();
}
