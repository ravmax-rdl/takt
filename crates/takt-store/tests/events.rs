use chrono::{TimeZone, Utc};
use serde_json::json;
use takt_google::model::{CalendarListEntry, EventFields, Item};
use takt_store::*;

fn ev(value: serde_json::Value) -> Item<EventFields> {
    Item::from_value(value).unwrap()
}

fn seeded() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.upsert_account("acc", "me@example.com").unwrap();
    let cal: Item<CalendarListEntry> = Item::from_value(
        json!({"id": "cal", "summary": "Me", "selected": true, "timeZone": "Europe/London"}),
    )
    .unwrap();
    store.apply_calendar_list_page("acc", &[cal], None).unwrap();
    store
}

fn standup() -> serde_json::Value {
    json!({
        "id": "s1", "status": "confirmed", "summary": "Standup",
        "start": {"dateTime": "2026-10-19T09:00:00+01:00"}, "end": {"dateTime": "2026-10-19T09:30:00+01:00"}
    })
}

fn weekly_master() -> serde_json::Value {
    json!({
        "id": "m", "status": "confirmed", "summary": "1:1",
        "start": {"dateTime": "2026-10-05T10:00:00+01:00", "timeZone": "Europe/London"},
        "end": {"dateTime": "2026-10-05T10:30:00+01:00", "timeZone": "Europe/London"},
        "recurrence": ["RRULE:FREQ=WEEKLY"]
    })
}

#[test]
fn single_event_is_stored_with_token_and_reported() {
    let mut store = seeded();
    let changes = store
        .apply_events_page("acc", "cal", &[ev(standup())], Some("t1"))
        .unwrap();
    assert_eq!(
        store.events_sync_token("acc", "cal").unwrap().as_deref(),
        Some("t1")
    );
    assert!(changes.calendars.contains("cal"));
    assert!(changes.affects(
        Utc.with_ymd_and_hms(2026, 10, 19, 8, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 10, 19, 8, 10, 0).unwrap()
    ));
    assert!(!changes.everything);
    assert_eq!(
        store.event_raw("acc", "cal", "s1").unwrap(),
        Some(standup())
    );
}

#[test]
fn page_without_token_keeps_previous_token() {
    let mut store = seeded();
    store
        .apply_events_page("acc", "cal", &[], Some("t0"))
        .unwrap();
    store
        .apply_events_page("acc", "cal", &[ev(standup())], None)
        .unwrap();
    assert_eq!(
        store.events_sync_token("acc", "cal").unwrap().as_deref(),
        Some("t0")
    );
}

#[test]
fn applying_a_page_twice_gives_the_same_state() {
    let mut store = seeded();
    let page = [ev(standup()), ev(weekly_master())];
    store.apply_events_page("acc", "cal", &page, None).unwrap();
    store
        .apply_events_page("acc", "cal", &page, Some("t1"))
        .unwrap();
    assert_eq!(store.event_ids("acc", "cal").unwrap(), ["m", "s1"]);
}

#[test]
fn cancelled_single_is_deleted_and_old_time_reported() {
    let mut store = seeded();
    store
        .apply_events_page("acc", "cal", &[ev(standup())], None)
        .unwrap();
    let changes = store
        .apply_events_page(
            "acc",
            "cal",
            &[ev(json!({"id": "s1", "status": "cancelled"}))],
            None,
        )
        .unwrap();
    assert!(store.event_ids("acc", "cal").unwrap().is_empty());
    assert!(changes.affects(
        Utc.with_ymd_and_hms(2026, 10, 19, 8, 0, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 10, 19, 8, 10, 0).unwrap()
    ));
}

#[test]
fn minimal_cancelled_master_removes_series_and_exceptions() {
    let mut store = seeded();
    let exception = json!({
        "id": "m_20261012T090000Z", "status": "confirmed", "summary": "1:1 (moved)", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-12T10:00:00+01:00", "timeZone": "Europe/London"},
        "start": {"dateTime": "2026-10-13T10:00:00+01:00"}, "end": {"dateTime": "2026-10-13T10:30:00+01:00"}
    });
    store
        .apply_events_page("acc", "cal", &[ev(weekly_master()), ev(exception)], None)
        .unwrap();
    assert_eq!(store.event_ids("acc", "cal").unwrap().len(), 2);
    store
        .apply_events_page(
            "acc",
            "cal",
            &[ev(json!({"id": "m", "status": "cancelled"}))],
            None,
        )
        .unwrap();
    assert!(store.event_ids("acc", "cal").unwrap().is_empty());
}

#[test]
fn cancelled_exception_is_kept() {
    let mut store = seeded();
    let cancelled = json!({
        "id": "m_20261012T090000Z", "status": "cancelled", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-12T10:00:00+01:00", "timeZone": "Europe/London"}
    });
    store
        .apply_events_page("acc", "cal", &[ev(weekly_master()), ev(cancelled)], None)
        .unwrap();
    assert_eq!(
        store.event_ids("acc", "cal").unwrap(),
        ["m", "m_20261012T090000Z"]
    );
}

#[test]
fn endless_series_reports_an_unbounded_span() {
    let mut store = seeded();
    let changes = store
        .apply_events_page("acc", "cal", &[ev(weekly_master())], None)
        .unwrap();
    assert_eq!(changes.span.unwrap().end, None);
}

#[test]
fn reset_calendar_events_clears_rows_and_token() {
    let mut store = seeded();
    store
        .apply_events_page("acc", "cal", &[ev(standup())], Some("t1"))
        .unwrap();
    store.reset_calendar_events("acc", "cal").unwrap();
    assert!(store.event_ids("acc", "cal").unwrap().is_empty());
    assert_eq!(store.events_sync_token("acc", "cal").unwrap(), None);
}

#[test]
fn calendar_list_update_keeps_events_and_token() {
    let mut store = seeded();
    store
        .apply_events_page("acc", "cal", &[ev(standup())], Some("t1"))
        .unwrap();
    let recoloured: Item<CalendarListEntry> = Item::from_value(
        json!({"id": "cal", "summary": "Me", "selected": true, "backgroundColor": "#000000"}),
    )
    .unwrap();
    store
        .apply_calendar_list_page("acc", &[recoloured], None)
        .unwrap();
    assert_eq!(store.event_ids("acc", "cal").unwrap(), ["s1"]);
    assert_eq!(
        store.events_sync_token("acc", "cal").unwrap().as_deref(),
        Some("t1")
    );
}

#[test]
fn removing_a_calendar_removes_its_events() {
    let mut store = seeded();
    store
        .apply_events_page("acc", "cal", &[ev(standup())], None)
        .unwrap();
    store.remove_calendar("acc", "cal").unwrap();
    assert!(store.event_ids("acc", "cal").unwrap().is_empty());
}
