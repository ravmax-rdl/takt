//! `AccountEngine::write` against a fake Google.

mod common;

use chrono::{TimeZone, Utc};
use common::*;
use serde_json::{Value, json};
use takt_google::model::{CalendarListEntry, Item};
use takt_store::When;
use takt_sync::{InstanceOf, SyncEvent, WriteError, WriteOp, WriteOutcome, WriteRequest};
use wiremock::matchers::{body_json, body_partial_json, header, method, path, query_param};
use wiremock::{Mock, ResponseTemplate};

const EVENTS: &str = "/calendar/v3/calendars/cal/events";

/// A fake with calendars "cal" and "work" in the store.
async fn fake_with_calendars() -> Fake {
    let f = fake().await;
    let calendars: Vec<Item<CalendarListEntry>> = ["cal", "work"]
        .iter()
        .map(|id| Item::from_value(json!({"id": id, "selected": true, "timeZone": "UTC"})).unwrap())
        .collect();
    f.store
        .lock()
        .unwrap()
        .apply_calendar_list_page("acc", &calendars, None)
        .unwrap();
    f
}

fn request(op: WriteOp) -> WriteRequest {
    WriteRequest {
        id: 7,
        account_id: "acc".into(),
        calendar_id: "cal".into(),
        op,
        notify_guests: false,
    }
}

fn versioned(id: &str, etag: &str, summary: &str) -> Value {
    let mut value = event(id, 20);
    value["etag"] = etag.into();
    value["summary"] = summary.into();
    value
}

fn seed(f: &Fake, calendar: &str, value: Value) {
    f.store
        .lock()
        .unwrap()
        .apply_event("acc", calendar, &Item::from_value(value).unwrap())
        .unwrap();
    while f.events.try_recv().is_ok() {}
}

fn raw(f: &Fake, calendar: &str, id: &str) -> Option<Value> {
    f.store
        .lock()
        .unwrap()
        .event_raw("acc", calendar, id)
        .unwrap()
}

fn changed(f: &Fake) -> usize {
    let mut count = 0;
    while let Ok(event) = f.events.try_recv() {
        if matches!(event, SyncEvent::Changed { .. }) {
            count += 1;
        }
    }
    count
}

fn update(patch: Value, base: Value, etag: &str) -> WriteOp {
    WriteOp::Update {
        event_id: "e1".into(),
        etag: Some(etag.into()),
        patch,
        base,
        move_to: None,
    }
}

async fn patches(f: &Fake) -> usize {
    f.server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == "PATCH")
        .count()
}

#[tokio::test]
async fn create_chooses_an_id_and_stores_what_google_returns() {
    let f = fake_with_calendars().await;
    Mock::given(method("POST"))
        .and(path(EVENTS))
        .and(query_param("sendUpdates", "none"))
        .and(body_partial_json(json!({"summary": "Lunch"})))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = request.body_json().unwrap();
            let id = body["id"].as_str().expect("the engine adds an id");
            assert_eq!(id.len(), 26);
            ok(versioned(id, "\"1\"", "Lunch"))
        })
        .expect(1)
        .mount(&f.server)
        .await;

    let body = json!({"summary": "Lunch", "start": {"dateTime": "2026-10-20T09:00:00Z"}});
    let outcome = f
        .engine
        .write(&request(WriteOp::Create { body }))
        .await
        .unwrap();
    assert_eq!(outcome.calendar_id, "cal");
    assert_eq!(ids(&f.store), [outcome.event_id]);
    assert_eq!(changed(&f), 1);
}

#[tokio::test]
async fn a_repeated_insert_reads_the_event_it_already_made() {
    let f = fake_with_calendars().await;
    Mock::given(method("POST"))
        .and(path(EVENTS))
        .respond_with(ResponseTemplate::new(409))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{EVENTS}/myid1")))
        .respond_with(ok(versioned("myid1", "\"1\"", "Lunch")))
        .expect(1)
        .mount(&f.server)
        .await;
    let body = json!({"id": "myid1", "summary": "Lunch"});
    let outcome = f
        .engine
        .write(&request(WriteOp::Create { body }))
        .await
        .unwrap();
    assert_eq!(outcome.event_id, "myid1");
    assert_eq!(ids(&f.store), ["myid1"]);
}

#[tokio::test]
async fn update_patches_with_the_etag_and_stores_the_answer() {
    let f = fake_with_calendars().await;
    seed(&f, "cal", versioned("e1", "\"1\"", "Lunch"));
    let patch = json!({"summary": "Late lunch"});
    Mock::given(method("PATCH"))
        .and(path(format!("{EVENTS}/e1")))
        .and(header("if-match", "\"1\""))
        .and(body_json(&patch))
        .respond_with(ok(versioned("e1", "\"2\"", "Late lunch")))
        .expect(1)
        .mount(&f.server)
        .await;
    let outcome = f
        .engine
        .write(&request(update(
            patch,
            json!({"summary": "Lunch"}),
            "\"1\"",
        )))
        .await
        .unwrap();
    assert_eq!(
        outcome,
        WriteOutcome {
            calendar_id: "cal".into(),
            event_id: "e1".into()
        }
    );
    let stored = raw(&f, "cal", "e1").unwrap();
    assert_eq!(stored["summary"], "Late lunch");
    assert_eq!(stored["etag"], "\"2\"");
    assert_eq!(changed(&f), 1);
}

#[tokio::test]
async fn notifying_guests_asks_google_to_send_updates() {
    let f = fake_with_calendars().await;
    seed(&f, "cal", versioned("e1", "\"1\"", "Lunch"));
    Mock::given(method("PATCH"))
        .and(query_param("sendUpdates", "all"))
        .respond_with(ok(versioned("e1", "\"2\"", "Late lunch")))
        .expect(1)
        .mount(&f.server)
        .await;
    let mut write = request(update(json!({"summary": "Late lunch"}), json!({}), "\"1\""));
    write.notify_guests = true;
    f.engine.write(&write).await.unwrap();
}

/// Someone else changed the description; this edit changes the title.
#[tokio::test]
async fn a_stale_etag_is_retried_when_other_fields_changed() {
    let f = fake_with_calendars().await;
    seed(&f, "cal", versioned("e1", "\"1\"", "Lunch"));
    let mut remote = versioned("e1", "\"2\"", "Lunch");
    remote["description"] = "bring cake".into();
    Mock::given(method("PATCH"))
        .and(header("if-match", "\"1\""))
        .respond_with(ResponseTemplate::new(412))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{EVENTS}/e1")))
        .respond_with(ok(remote.clone()))
        .expect(1)
        .mount(&f.server)
        .await;
    let mut merged = remote;
    merged["etag"] = "\"3\"".into();
    merged["summary"] = "Late lunch".into();
    Mock::given(method("PATCH"))
        .and(header("if-match", "\"2\""))
        .respond_with(ok(merged))
        .expect(1)
        .mount(&f.server)
        .await;

    f.engine
        .write(&request(update(
            json!({"summary": "Late lunch"}),
            json!({"summary": "Lunch"}),
            "\"1\"",
        )))
        .await
        .unwrap();
    let stored = raw(&f, "cal", "e1").unwrap();
    assert_eq!(stored["summary"], "Late lunch");
    assert_eq!(stored["description"], "bring cake");
    assert_eq!(stored["etag"], "\"3\"");
}

/// Someone else changed the title this edit also changes.
#[tokio::test]
async fn a_stale_etag_is_a_conflict_when_the_same_field_changed() {
    let f = fake_with_calendars().await;
    seed(&f, "cal", versioned("e1", "\"1\"", "Lunch"));
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(412))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("{EVENTS}/e1")))
        .respond_with(ok(versioned("e1", "\"2\"", "Brunch")))
        .mount(&f.server)
        .await;

    let result = f
        .engine
        .write(&request(update(
            json!({"summary": "Late lunch"}),
            json!({"summary": "Lunch"}),
            "\"1\"",
        )))
        .await;
    assert_eq!(result, Err(WriteError::Conflict));
    assert_eq!(patches(&f).await, 1, "the patch is not sent again");
    let stored = raw(&f, "cal", "e1").unwrap();
    assert_eq!(
        stored["summary"], "Brunch",
        "the cache holds Google's version"
    );
    assert_eq!(changed(&f), 1);
}

/// The first attempt reached Google but its answer was lost.
#[tokio::test]
async fn a_stale_etag_is_fine_when_google_already_has_the_change() {
    let f = fake_with_calendars().await;
    seed(&f, "cal", versioned("e1", "\"1\"", "Lunch"));
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(412))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ok(versioned("e1", "\"2\"", "Late lunch")))
        .mount(&f.server)
        .await;
    f.engine
        .write(&request(update(
            json!({"summary": "Late lunch"}),
            json!({"summary": "Lunch"}),
            "\"1\"",
        )))
        .await
        .unwrap();
    assert_eq!(patches(&f).await, 1);
    assert_eq!(raw(&f, "cal", "e1").unwrap()["etag"], "\"2\"");
}

#[tokio::test]
async fn updating_an_event_deleted_elsewhere_drops_it() {
    let f = fake_with_calendars().await;
    seed(&f, "cal", versioned("e1", "\"1\"", "Lunch"));
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&f.server)
        .await;
    let result = f
        .engine
        .write(&request(update(
            json!({"summary": "x"}),
            json!({}),
            "\"1\"",
        )))
        .await;
    assert_eq!(result, Err(WriteError::Gone));
    assert!(ids(&f.store).is_empty());
}

#[tokio::test]
async fn a_calendar_change_patches_then_moves() {
    let f = fake_with_calendars().await;
    seed(&f, "cal", versioned("e1", "\"1\"", "Lunch"));
    Mock::given(method("PATCH"))
        .and(path(format!("{EVENTS}/e1")))
        .respond_with(ok(versioned("e1", "\"2\"", "Late lunch")))
        .expect(1)
        .mount(&f.server)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("{EVENTS}/e1/move")))
        .and(query_param("destination", "work"))
        .respond_with(ok(versioned("e1", "\"3\"", "Late lunch")))
        .expect(1)
        .mount(&f.server)
        .await;
    let outcome = f
        .engine
        .write(&request(WriteOp::Update {
            event_id: "e1".into(),
            etag: Some("\"1\"".into()),
            patch: json!({"summary": "Late lunch"}),
            base: json!({"summary": "Lunch"}),
            move_to: Some("work".into()),
        }))
        .await
        .unwrap();
    assert_eq!(outcome.calendar_id, "work");
    assert!(raw(&f, "cal", "e1").is_none());
    assert_eq!(raw(&f, "work", "e1").unwrap()["etag"], "\"3\"");
}

#[tokio::test]
async fn a_calendar_change_alone_sends_no_patch() {
    let f = fake_with_calendars().await;
    seed(&f, "cal", versioned("e1", "\"1\"", "Lunch"));
    Mock::given(method("POST"))
        .and(path(format!("{EVENTS}/e1/move")))
        .respond_with(ok(versioned("e1", "\"2\"", "Lunch")))
        .expect(1)
        .mount(&f.server)
        .await;
    f.engine
        .write(&request(WriteOp::Update {
            event_id: "e1".into(),
            etag: Some("\"1\"".into()),
            patch: json!({}),
            base: json!({}),
            move_to: Some("work".into()),
        }))
        .await
        .unwrap();
    assert_eq!(patches(&f).await, 0);
    assert!(raw(&f, "work", "e1").is_some());
}

#[tokio::test]
async fn delete_removes_the_event_even_if_google_already_did() {
    for status in [204, 410, 404] {
        let f = fake_with_calendars().await;
        seed(&f, "cal", versioned("e1", "\"1\"", "Lunch"));
        Mock::given(method("DELETE"))
            .and(path(format!("{EVENTS}/e1")))
            .respond_with(ResponseTemplate::new(status))
            .expect(1)
            .mount(&f.server)
            .await;
        f.engine
            .write(&request(WriteOp::Delete {
                event_id: "e1".into(),
                instance: None,
            }))
            .await
            .unwrap();
        assert!(ids(&f.store).is_empty(), "after a {status}");
        assert_eq!(changed(&f), 1);
    }
}

#[tokio::test]
async fn deleting_an_occurrence_hides_it_and_keeps_the_series() {
    let f = fake_with_calendars().await;
    let mut series = event("m", 5);
    series["recurrence"] = json!(["RRULE:FREQ=WEEKLY"]);
    seed(&f, "cal", series);
    Mock::given(method("DELETE"))
        .and(path(format!("{EVENTS}/m_20261012T090000Z")))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&f.server)
        .await;
    f.engine
        .write(&request(WriteOp::Delete {
            event_id: "m_20261012T090000Z".into(),
            instance: Some(InstanceOf {
                series_id: "m".into(),
                original_start: When::At(Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap()),
            }),
        }))
        .await
        .unwrap();
    assert_eq!(ids(&f.store), ["m", "m_20261012T090000Z"]);
    let cancelled = raw(&f, "cal", "m_20261012T090000Z").unwrap();
    assert_eq!(cancelled["status"], "cancelled");
    assert_eq!(cancelled["recurringEventId"], "m");
}

#[tokio::test]
async fn a_refused_write_reports_googles_message_and_changes_nothing() {
    let f = fake_with_calendars().await;
    seed(&f, "cal", versioned("e1", "\"1\"", "Lunch"));
    Mock::given(method("PATCH"))
        .respond_with(ResponseTemplate::new(400).set_body_json(
            json!({"error": {"code": 400, "message": "The specified time range is empty."}}),
        ))
        .mount(&f.server)
        .await;
    let result = f
        .engine
        .write(&request(update(json!({"end": {}}), json!({}), "\"1\"")))
        .await;
    assert_eq!(
        result,
        Err(WriteError::Rejected(
            "HTTP 400: The specified time range is empty.".into()
        ))
    );
    assert_eq!(raw(&f, "cal", "e1").unwrap()["etag"], "\"1\"");
    assert_eq!(changed(&f), 0);
}

#[tokio::test]
async fn a_write_without_a_connection_is_offline() {
    // Nothing listens on port 1.
    let (store, _events, engine) = engine_for("http://127.0.0.1:1", None);
    let cal: Item<CalendarListEntry> = Item::from_value(json!({"id": "cal"})).unwrap();
    store
        .lock()
        .unwrap()
        .apply_calendar_list_page("acc", &[cal], None)
        .unwrap();
    let result = engine
        .write(&request(WriteOp::Create {
            body: json!({"summary": "Lunch"}),
        }))
        .await;
    assert_eq!(result, Err(WriteError::Offline));
    assert!(ids(&store).is_empty());
}
