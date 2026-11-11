//! What the sync thread stores after Google accepts a write.

use chrono::{NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
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
        "id": "s1", "etag": "\"1\"", "status": "confirmed", "summary": "Standup",
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

fn october(store: &Store) -> Vec<Occurrence> {
    let d = |day| NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
    store
        .occurrences(
            &local_days(d(1), d(31), Tz::Europe__London),
            Tz::Europe__London,
        )
        .unwrap()
}

#[test]
fn a_written_event_is_stored_and_keeps_the_sync_token() {
    let mut store = seeded();
    store.apply_events_page("acc", "cal", &[], Some("t0")).unwrap();
    let changes = store.apply_event("acc", "cal", &ev(standup())).unwrap();
    assert!(changes.calendars.contains("cal"));
    assert_eq!(
        store.events_sync_token("acc", "cal").unwrap().as_deref(),
        Some("t0")
    );
    assert_eq!(store.event_raw("acc", "cal", "s1").unwrap(), Some(standup()));
}

#[test]
fn a_moved_event_reports_both_its_old_and_new_time() {
    let mut store = seeded();
    store.apply_event("acc", "cal", &ev(standup())).unwrap();
    let mut later = standup();
    later["start"]["dateTime"] = "2026-10-22T14:00:00+01:00".into();
    later["end"]["dateTime"] = "2026-10-22T14:30:00+01:00".into();
    let changes = store.apply_event("acc", "cal", &ev(later)).unwrap();
    let at = |day, hour| Utc.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap();
    assert!(changes.affects(at(19, 8), at(19, 9)), "where it was");
    assert!(changes.affects(at(22, 13), at(22, 14)), "where it is");
}

#[test]
fn deleting_a_single_event_removes_its_row() {
    let mut store = seeded();
    store.apply_event("acc", "cal", &ev(standup())).unwrap();
    let changes = store
        .mark_event_deleted("acc", "cal", "s1", None)
        .unwrap();
    assert!(changes.calendars.contains("cal"));
    assert!(store.event_ids("acc", "cal").unwrap().is_empty());
}

#[test]
fn deleting_an_unknown_event_changes_nothing() {
    let mut store = seeded();
    let changes = store
        .mark_event_deleted("acc", "cal", "nope", None)
        .unwrap();
    assert!(changes.is_empty());
}

#[test]
fn deleting_a_generated_occurrence_leaves_a_cancelled_exception() {
    let mut store = seeded();
    store.apply_event("acc", "cal", &ev(weekly_master())).unwrap();
    assert_eq!(october(&store).len(), 4);

    let original = When::At(Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap());
    store
        .mark_event_deleted("acc", "cal", "m_20261012T090000Z", Some(("m", original)))
        .unwrap();
    let left: Vec<_> = october(&store).into_iter().map(|o| o.instance_id).collect();
    assert_eq!(
        left,
        ["m_20261005T090000Z", "m_20261019T090000Z", "m_20261026T100000Z"]
    );
    assert_eq!(
        store.event_ids("acc", "cal").unwrap(),
        ["m", "m_20261012T090000Z"]
    );
}

#[test]
fn deleting_a_moved_occurrence_keeps_its_row_as_cancelled() {
    let mut store = seeded();
    let moved = json!({
        "id": "m_20261012T090000Z", "status": "confirmed", "summary": "1:1 (moved)", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-12T10:00:00+01:00", "timeZone": "Europe/London"},
        "start": {"dateTime": "2026-10-13T10:00:00+01:00"}, "end": {"dateTime": "2026-10-13T10:30:00+01:00"}
    });
    store
        .apply_events_page("acc", "cal", &[ev(weekly_master()), ev(moved)], None)
        .unwrap();
    assert_eq!(october(&store).len(), 4);

    store
        .mark_event_deleted("acc", "cal", "m_20261012T090000Z", None)
        .unwrap();
    assert_eq!(october(&store).len(), 3);
    let raw = store
        .event_raw("acc", "cal", "m_20261012T090000Z")
        .unwrap()
        .unwrap();
    assert_eq!(raw["status"], "cancelled");
    assert_eq!(raw["summary"], "1:1 (moved)", "the rest of the row is kept");
}

#[test]
fn deleting_an_all_day_occurrence_uses_its_date() {
    let mut store = seeded();
    let master = json!({
        "id": "b", "summary": "Bins",
        "start": {"date": "2026-10-06"}, "end": {"date": "2026-10-07"},
        "recurrence": ["RRULE:FREQ=WEEKLY"]
    });
    store.apply_event("acc", "cal", &ev(master)).unwrap();
    let original = When::Date(NaiveDate::from_ymd_opt(2026, 10, 13).unwrap());
    store
        .mark_event_deleted("acc", "cal", "b_20261013", Some(("b", original)))
        .unwrap();
    let left: Vec<_> = october(&store).into_iter().map(|o| o.instance_id).collect();
    assert_eq!(left, ["b_20261006", "b_20261020", "b_20261027"]);
}

#[test]
fn deleting_a_series_removes_its_exceptions_too() {
    let mut store = seeded();
    let cancelled = json!({
        "id": "m_20261012T090000Z", "status": "cancelled", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-12T10:00:00+01:00", "timeZone": "Europe/London"}
    });
    store
        .apply_events_page("acc", "cal", &[ev(weekly_master()), ev(cancelled)], None)
        .unwrap();
    store.mark_event_deleted("acc", "cal", "m", None).unwrap();
    assert!(store.event_ids("acc", "cal").unwrap().is_empty());
}
