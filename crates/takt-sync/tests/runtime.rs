mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use common::*;
use serde_json::json;
use takt_google::{Endpoints, MemoryTokenStore, RetryPolicy, TokenStore, Url};
use takt_store::{AccountStatus, Store};
use takt_sync::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Rig {
    server: MockServer,
    dir: tempfile::TempDir,
    tokens: Arc<MemoryTokenStore>,
}

impl Rig {
    async fn new() -> Self {
        Self {
            server: MockServer::start().await,
            dir: tempfile::tempdir().unwrap(),
            tokens: Arc::new(MemoryTokenStore::default()),
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
        start(SyncConfig {
            db_path: self.db(),
            endpoints: Endpoints::rooted_at(&self.server.uri()),
            oauth: oauth(),
            tokens: self.tokens.clone(),
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
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(ok(json!({"access_token": "at", "expires_in": 3600})))
            .mount(&self.server)
            .await;
        mount_calendar_list(&self.server).await;
        mount_meta(&self.server).await;
        mount_no_tasks(&self.server).await;
        Mock::given(method("GET"))
            .and(path("/calendar/v3/calendars/cal/events"))
            .respond_with(ok(json!({"items": [], "nextSyncToken": "s"})))
            .mount(&self.server)
            .await;
    }
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
