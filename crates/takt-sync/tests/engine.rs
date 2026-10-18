mod common;

use chrono::NaiveDate;
use common::*;
use serde_json::json;
use takt_google::{Auth, Endpoints};
use takt_sync::SyncEvent;
use wiremock::matchers::{method, path, query_param, query_param_is_missing};
use wiremock::{Mock, ResponseTemplate};

const EVENTS: &str = "/calendar/v3/calendars/cal/events";

#[tokio::test]
async fn first_round_fills_the_store() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_meta(&f.server).await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .and(query_param_is_missing("syncToken"))
        .respond_with(ok(
            json!({"items": [event("e1", 20)], "nextSyncToken": "s1"}),
        ))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/users/@me/lists"))
        .respond_with(ok(json!({"items": [{"id": "L1", "title": "My Tasks"}]})))
        .mount(&f.server)
        .await;
    Mock::given(method("GET")).and(path("/tasks/v1/lists/L1/tasks")).and(query_param_is_missing("updatedMin"))
        .respond_with(ok(json!({"items": [{"id": "t1", "title": "Pay rent", "due": "2026-10-20T00:00:00.000Z", "updated": "2026-10-17T10:00:00.000Z"}]})))
        .mount(&f.server).await;

    f.engine.round(true).await.unwrap();

    assert_eq!(ids(&f.store), ["e1"]);
    let store = f.store.lock().unwrap();
    assert_eq!(
        store.events_sync_token("acc", "cal").unwrap().as_deref(),
        Some("s1")
    );
    assert_eq!(
        store.calendar_list_sync_token("acc").unwrap().as_deref(),
        Some("cl1")
    );
    assert_eq!(store.settings("acc").unwrap()["format24HourTime"], "true");
    let d = |day| NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
    assert_eq!(store.tasks_due(d(19), d(21)).unwrap().len(), 1);
    drop(store);
    let mut changed = 0;
    while let Ok(ev) = f.events.try_recv() {
        if matches!(ev, SyncEvent::Changed { .. }) {
            changed += 1;
        }
    }
    assert!(
        changed >= 3,
        "calendar list, meta, events and tasks each report a change"
    );
}

#[tokio::test]
async fn incremental_round_sends_stored_tokens() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_no_tasks(&f.server).await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .and(query_param_is_missing("syncToken"))
        .respond_with(ok(
            json!({"items": [event("e1", 20)], "nextSyncToken": "s1"}),
        ))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .and(query_param("syncToken", "s1"))
        .respond_with(ok(
            json!({"items": [event("e2", 21)], "nextSyncToken": "s2"}),
        ))
        .mount(&f.server)
        .await;
    f.engine.round(false).await.unwrap();
    f.engine.round(false).await.unwrap();
    assert_eq!(ids(&f.store), ["e1", "e2"]);
    assert_eq!(
        f.store
            .lock()
            .unwrap()
            .events_sync_token("acc", "cal")
            .unwrap()
            .as_deref(),
        Some("s2")
    );
}

#[tokio::test]
async fn gone_on_events_resets_and_full_syncs() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_no_tasks(&f.server).await;
    {
        // The calendar-list upsert in the round keeps this token (ON CONFLICT DO UPDATE).
        let mut store = f.store.lock().unwrap();
        let cal =
            takt_google::model::Item::from_value(json!({"id": "cal", "selected": true})).unwrap();
        store.apply_calendar_list_page("acc", &[cal], None).unwrap();
        let old = takt_google::model::Item::from_value(event("old", 1)).unwrap();
        store
            .apply_events_page("acc", "cal", &[old], Some("stale"))
            .unwrap();
    }
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .and(query_param("syncToken", "stale"))
        .respond_with(ResponseTemplate::new(410))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .and(query_param_is_missing("syncToken"))
        .respond_with(ok(
            json!({"items": [event("fresh", 22)], "nextSyncToken": "s9"}),
        ))
        .mount(&f.server)
        .await;
    f.engine.round(false).await.unwrap();
    assert_eq!(ids(&f.store), ["fresh"]);
    assert_eq!(
        f.store
            .lock()
            .unwrap()
            .events_sync_token("acc", "cal")
            .unwrap()
            .as_deref(),
        Some("s9")
    );
}

#[tokio::test]
async fn calendar_that_404s_is_removed() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_no_tasks(&f.server).await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .respond_with(ResponseTemplate::new(404))
        .mount(&f.server)
        .await;
    f.engine.round(false).await.unwrap();
    assert!(
        f.store
            .lock()
            .unwrap()
            .calendars(Some("acc"))
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn interrupted_paging_resumes_from_the_old_token() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_no_tasks(&f.server).await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ok(
            json!({"items": [event("e1", 20)], "nextPageToken": "p2"}),
        ))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .and(query_param("pageToken", "p2"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&f.server)
        .await;
    assert!(f.engine.round(false).await.is_err());
    assert_eq!(ids(&f.store), ["e1"]);
    assert_eq!(
        f.store
            .lock()
            .unwrap()
            .events_sync_token("acc", "cal")
            .unwrap(),
        None
    );

    f.server.reset().await;
    mount_calendar_list(&f.server).await;
    mount_no_tasks(&f.server).await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ok(
            json!({"items": [event("e1", 20)], "nextPageToken": "p2"}),
        ))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .and(query_param("pageToken", "p2"))
        .respond_with(ok(
            json!({"items": [event("e2", 21)], "nextSyncToken": "s1"}),
        ))
        .mount(&f.server)
        .await;
    f.engine.round(false).await.unwrap();
    assert_eq!(ids(&f.store), ["e1", "e2"]);
    assert_eq!(
        f.store
            .lock()
            .unwrap()
            .events_sync_token("acc", "cal")
            .unwrap()
            .as_deref(),
        Some("s1")
    );
}

#[tokio::test]
async fn unreachable_google_is_offline() {
    let (_store, _events, engine) = engine_for("http://127.0.0.1:9", None);
    let err = engine.round(false).await.unwrap_err();
    assert!(err.is_offline(), "{err}");
}

#[tokio::test]
async fn revoked_refresh_token_needs_reauth() {
    let f = fake().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant"})))
        .mount(&f.server)
        .await;
    let endpoints = Endpoints::rooted_at(&f.server.uri());
    let auth = Auth::new(reqwest::Client::new(), endpoints.token, oauth(), "revoked");
    let (_store, _events, engine) = engine_for(&f.server.uri(), Some(auth));
    let err = engine.round(false).await.unwrap_err();
    assert!(err.needs_reauth(), "{err}");
}

#[tokio::test]
async fn forbidden_tasks_api_is_skipped() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .respond_with(ok(json!({"items": [], "nextSyncToken": "s1"})))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/users/@me/lists"))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"error": {"errors": [{"reason": "forbidden"}]}})),
        )
        .mount(&f.server)
        .await;
    f.engine.round(false).await.unwrap();
}
