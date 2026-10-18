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

fn at(day: u32, hour: u32, minute: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, day, hour, minute, 0)
        .unwrap()
}

fn moved_exception() -> serde_json::Value {
    json!({
        "id": "m_20261012T090000Z", "status": "confirmed", "summary": "1:1 (moved)", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-12T10:00:00+01:00", "timeZone": "Europe/London"},
        "start": {"dateTime": "2026-10-13T10:00:00+01:00"}, "end": {"dateTime": "2026-10-13T10:30:00+01:00"}
    })
}

#[test]
fn moved_exception_reports_its_original_slot() {
    let mut store = seeded();
    store
        .apply_events_page("acc", "cal", &[ev(weekly_master())], None)
        .unwrap();
    let changes = store
        .apply_events_page("acc", "cal", &[ev(moved_exception())], None)
        .unwrap();
    assert!(
        changes.affects(at(12, 9, 10), at(12, 9, 20)),
        "original slot"
    );
    assert!(changes.affects(at(13, 9, 10), at(13, 9, 20)), "new slot");
    assert!(!changes.affects(at(12, 8, 0), at(12, 8, 50)));
}

#[test]
fn minimal_cancelled_instance_lasts_as_long_as_its_series() {
    let mut store = seeded();
    store
        .apply_events_page("acc", "cal", &[ev(weekly_master())], None)
        .unwrap();
    let cancelled = json!({
        "id": "m_20261012T090000Z", "status": "cancelled", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-12T10:00:00+01:00", "timeZone": "Europe/London"}
    });
    let changes = store
        .apply_events_page("acc", "cal", &[ev(cancelled.clone())], None)
        .unwrap();
    assert!(changes.affects(at(12, 9, 20), at(12, 9, 25)));
    // Its stored span has the series' length too, so a later change reports all of it.
    let changes = store
        .apply_events_page("acc", "cal", &[ev(cancelled)], None)
        .unwrap();
    assert!(changes.affects(at(12, 9, 20), at(12, 9, 25)));
}

#[test]
fn cancelled_series_reports_exceptions_moved_past_its_end() {
    let mut store = seeded();
    let mut master = weekly_master();
    master["recurrence"] = json!(["RRULE:FREQ=WEEKLY;COUNT=3"]);
    let moved = json!({
        "id": "m_20261019T090000Z", "status": "confirmed", "summary": "1:1 (moved)", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-19T10:00:00+01:00", "timeZone": "Europe/London"},
        "start": {"dateTime": "2026-11-02T10:00:00Z"}, "end": {"dateTime": "2026-11-02T10:30:00Z"}
    });
    store
        .apply_events_page("acc", "cal", &[ev(master), ev(moved)], None)
        .unwrap();
    let changes = store
        .apply_events_page(
            "acc",
            "cal",
            &[ev(json!({"id": "m", "status": "cancelled"}))],
            None,
        )
        .unwrap();
    assert!(store.event_ids("acc", "cal").unwrap().is_empty());
    assert!(changes.affects(
        Utc.with_ymd_and_hms(2026, 11, 2, 10, 10, 0).unwrap(),
        Utc.with_ymd_and_hms(2026, 11, 2, 10, 20, 0).unwrap()
    ));
}

#[test]
fn exception_arriving_before_its_master() {
    let mut store = seeded();
    let changes = store
        .apply_events_page("acc", "cal", &[ev(moved_exception())], None)
        .unwrap();
    // No series yet: the original slot is reported with the exception's own length.
    assert!(changes.affects(at(12, 9, 10), at(12, 9, 20)));
    assert!(changes.affects(at(13, 9, 10), at(13, 9, 20)));
    store
        .apply_events_page("acc", "cal", &[ev(weekly_master())], None)
        .unwrap();
    let week = local_days(
        chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
        chrono::NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(),
        chrono_tz::Europe::London,
    );
    let got = store.occurrences(&week, chrono_tz::Europe::London).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].instance_id, "m_20261012T090000Z");
    assert_eq!(got[0].start, When::At(at(13, 9, 0)));
}

#[test]
fn cancelled_instance_before_its_master_in_the_same_page() {
    let mut store = seeded();
    let cancelled = json!({
        "id": "m_20261012T090000Z", "status": "cancelled", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-12T10:00:00+01:00", "timeZone": "Europe/London"}
    });
    store
        .apply_events_page("acc", "cal", &[ev(cancelled), ev(weekly_master())], None)
        .unwrap();
    let days = local_days(
        chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap(),
        chrono::NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
        chrono_tz::Europe::London,
    );
    let got = store.occurrences(&days, chrono_tz::Europe::London).unwrap();
    let starts: Vec<_> = got.iter().map(|o| o.start).collect();
    assert_eq!(starts, [When::At(at(19, 9, 0))]);
}
