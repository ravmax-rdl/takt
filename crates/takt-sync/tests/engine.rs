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
    let store = f.store.lock().unwrap();
    assert!(store.calendars(Some("acc")).unwrap().is_empty());
    assert_eq!(
        store.calendar_list_sync_token("acc").unwrap(),
        None,
        "the next round refetches the full calendar list"
    );
}

#[tokio::test]
async fn calendar_over_quota_is_kept() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_no_tasks(&f.server).await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .respond_with(
            ResponseTemplate::new(403)
                .set_body_json(json!({"error": {"errors": [{"reason": "dailyLimitExceeded"}]}})),
        )
        .mount(&f.server)
        .await;
    assert!(f.engine.round(false).await.is_err());
    let store = f.store.lock().unwrap();
    assert_eq!(store.calendars(Some("acc")).unwrap().len(), 1);
    assert_eq!(
        store.calendar_list_sync_token("acc").unwrap().as_deref(),
        Some("cl1")
    );
}

async fn mount_one_task(server: &wiremock::MockServer) {
    Mock::given(method("GET"))
        .and(path("/tasks/v1/users/@me/lists"))
        .respond_with(ok(json!({"items": [{"id": "L1", "title": "My Tasks"}]})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/lists/L1/tasks"))
        .respond_with(ok(json!({"items": [
            {"id": "t1", "title": "Pay rent", "due": "2026-10-20T00:00:00.000Z", "updated": "2026-10-17T10:00:00.000Z"}
        ]})))
        .mount(server)
        .await;
}

fn tasks_due(f: &Fake) -> usize {
    let d = |day| NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
    f.store
        .lock()
        .unwrap()
        .tasks_due(d(19), d(21))
        .unwrap()
        .len()
}

#[tokio::test]
async fn failing_calendar_does_not_stop_tasks() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_one_task(&f.server).await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .respond_with(ResponseTemplate::new(500))
        .mount(&f.server)
        .await;
    let err = f.engine.round(false).await.unwrap_err();
    assert!(!err.is_offline() && !err.needs_reauth(), "{err}");
    assert_eq!(tasks_due(&f), 1);
}

#[tokio::test]
async fn failing_meta_does_not_stop_events_or_tasks() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_one_task(&f.server).await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/colors"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .respond_with(ok(
            json!({"items": [event("e1", 20)], "nextSyncToken": "s1"}),
        ))
        .mount(&f.server)
        .await;
    assert!(f.engine.round(true).await.is_err());
    assert_eq!(ids(&f.store), ["e1"]);
    assert_eq!(tasks_due(&f), 1);
    let hour = std::time::Duration::from_secs(3600);
    assert!(f.engine.meta_due(hour), "meta is retried next round");

    f.server.reset().await;
    mount_calendar_list(&f.server).await;
    mount_meta(&f.server).await;
    mount_no_tasks(&f.server).await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .respond_with(ok(json!({"items": [], "nextSyncToken": "s2"})))
        .mount(&f.server)
        .await;
    f.engine.round(true).await.unwrap();
    assert!(!f.engine.meta_due(hour));
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

async fn mount_task_pages(server: &wiremock::MockServer, second: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .respond_with(ok(json!({"items": [], "nextSyncToken": "s1"})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/users/@me/lists"))
        .respond_with(ok(json!({"items": [{"id": "L1", "title": "My Tasks"}]})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/lists/L1/tasks"))
        .and(query_param_is_missing("pageToken"))
        .respond_with(ok(json!({
            "items": [{"id": "t1", "title": "Pay rent", "updated": "2026-10-17T11:00:00.000Z"}],
            "nextPageToken": "p2"
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/lists/L1/tasks"))
        .and(query_param("pageToken", "p2"))
        .respond_with(second)
        .mount(server)
        .await;
}

#[tokio::test]
async fn interrupted_task_paging_resumes_from_the_old_checkpoint() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    mount_task_pages(&f.server, ResponseTemplate::new(500)).await;
    assert!(f.engine.round(false).await.is_err());
    assert_eq!(
        f.store
            .lock()
            .unwrap()
            .task_checkpoint("acc", "L1")
            .unwrap(),
        None
    );

    f.server.reset().await;
    mount_calendar_list(&f.server).await;
    mount_task_pages(
        &f.server,
        ok(json!({"items": [{"id": "t2", "title": "Call", "updated": "2026-10-17T09:00:00.000Z"}]})),
    )
    .await;
    f.engine.round(false).await.unwrap();
    let task_requests: Vec<_> = f
        .server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/tasks/v1/lists/L1/tasks")
        .collect();
    assert_eq!(task_requests.len(), 2);
    assert!(
        task_requests
            .iter()
            .all(|r| !r.url.query_pairs().any(|(k, _)| k == "updatedMin")),
        "the retry lists everything again"
    );
    assert_eq!(
        f.store
            .lock()
            .unwrap()
            .task_checkpoint("acc", "L1")
            .unwrap(),
        Some(
            chrono::DateTime::parse_from_rfc3339("2026-10-17T11:00:00Z")
                .unwrap()
                .to_utc()
        )
    );
}

fn forbidden(reason: &str) -> ResponseTemplate {
    ResponseTemplate::new(403).set_body_json(json!({"error": {"errors": [{"reason": reason}]}}))
}

fn seed_calendar(f: &Fake) {
    let cal = takt_google::model::Item::from_value(
        json!({"id": "cal", "summary": "Me", "selected": true, "timeZone": "UTC"}),
    )
    .unwrap();
    f.store
        .lock()
        .unwrap()
        .apply_calendar_list_page("acc", &[cal], Some("cl0"))
        .unwrap();
}

#[tokio::test]
async fn forbidden_calendar_list_keeps_calendars_and_skips_events() {
    let f = fake().await;
    seed_calendar(&f);
    mount_one_task(&f.server).await;
    Mock::given(method("GET"))
        .and(path("/calendar/v3/users/me/calendarList"))
        .respond_with(forbidden("insufficientPermissions"))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .respond_with(ResponseTemplate::new(404))
        .mount(&f.server)
        .await;
    let err = f.engine.round(false).await.unwrap_err();
    assert!(!err.is_offline() && !err.needs_reauth(), "{err}");
    assert_eq!(
        f.store
            .lock()
            .unwrap()
            .calendars(Some("acc"))
            .unwrap()
            .len(),
        1
    );
    let event_calls = f
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == EVENTS)
        .count();
    assert_eq!(event_calls, 0, "the events step is skipped");
    assert_eq!(tasks_due(&f), 1, "tasks still sync");
}

#[tokio::test]
async fn account_wide_403_on_events_keeps_the_calendar() {
    for reason in [
        "accessNotConfigured",
        "insufficientPermissions",
        "SERVICE_DISABLED",
        "ACCESS_TOKEN_SCOPE_INSUFFICIENT",
    ] {
        let f = fake().await;
        mount_calendar_list(&f.server).await;
        mount_no_tasks(&f.server).await;
        Mock::given(method("GET"))
            .and(path(EVENTS))
            .respond_with(forbidden(reason))
            .mount(&f.server)
            .await;
        assert!(f.engine.round(false).await.is_err(), "{reason}");
        let store = f.store.lock().unwrap();
        assert_eq!(store.calendars(Some("acc")).unwrap().len(), 1, "{reason}");
        assert_eq!(
            store.calendar_list_sync_token("acc").unwrap().as_deref(),
            Some("cl1"),
            "{reason}"
        );
    }
}

#[tokio::test]
async fn failing_task_list_does_not_stop_the_others() {
    let f = fake().await;
    mount_calendar_list(&f.server).await;
    Mock::given(method("GET"))
        .and(path(EVENTS))
        .respond_with(ok(json!({"items": [], "nextSyncToken": "s1"})))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/users/@me/lists"))
        .respond_with(ok(json!({"items": [
            {"id": "L1", "title": "Broken"},
            {"id": "L2", "title": "My Tasks"}
        ]})))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/lists/L1/tasks"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/tasks/v1/lists/L2/tasks"))
        .respond_with(ok(json!({"items": [
            {"id": "t1", "title": "Pay rent", "due": "2026-10-20T00:00:00.000Z", "updated": "2026-10-17T10:00:00.000Z"}
        ]})))
        .mount(&f.server)
        .await;
    let err = f.engine.round(false).await.unwrap_err();
    assert!(
        matches!(
            err,
            takt_sync::SyncError::Google(takt_google::GoogleError::Http { status: 500, .. })
        ),
        "{err}"
    );
    assert_eq!(tasks_due(&f), 1);
}
