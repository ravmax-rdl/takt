use chrono::NaiveDate;
use chrono_tz::Tz;
use serde_json::json;
use takt_google::model::{CalendarListEntry, EventFields, Item};
use takt_store::*;

fn ev(value: serde_json::Value) -> Item<EventFields> {
    Item::from_value(value).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn store_with(account: &str, calendar: serde_json::Value) -> Store {
    let mut store = Store::open_in_memory().unwrap();
    add(&mut store, account, calendar);
    store
}

fn add(store: &mut Store, account: &str, calendar: serde_json::Value) {
    store
        .upsert_account(account, &format!("{account}@example.com"))
        .unwrap();
    let cal: Item<CalendarListEntry> = Item::from_value(calendar).unwrap();
    store
        .apply_calendar_list_page(account, &[cal], None)
        .unwrap();
}

fn visible() -> serde_json::Value {
    json!({"id": "cal", "selected": true, "timeZone": "Europe/London"})
}

fn week_of_19_oct() -> Range {
    local_days(d(2026, 10, 19), d(2026, 10, 26), Tz::Europe__London)
}

#[test]
fn timed_single_inside_and_outside_range() {
    let mut store = store_with("acc", visible());
    store
        .apply_events_page("acc", "cal", &[
            ev(json!({"id": "in", "summary": "In", "start": {"dateTime": "2026-10-20T09:00:00Z"}, "end": {"dateTime": "2026-10-20T10:00:00Z"}})),
            ev(json!({"id": "out", "summary": "Out", "start": {"dateTime": "2026-11-20T09:00:00Z"}, "end": {"dateTime": "2026-11-20T10:00:00Z"}})),
        ], None)
        .unwrap();
    let got = store
        .occurrences(&week_of_19_oct(), Tz::Europe__London)
        .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].summary.as_deref(), Some("In"));
    assert_eq!(got[0].instance_id, "in");
}

#[test]
fn unselected_or_hidden_calendars_are_excluded() {
    let mut store = store_with("acc", json!({"id": "cal", "selected": false}));
    add(
        &mut store,
        "acc",
        json!({"id": "hid", "selected": true, "hidden": true}),
    );
    let e = json!({"id": "x", "start": {"dateTime": "2026-10-20T09:00:00Z"}, "end": {"dateTime": "2026-10-20T10:00:00Z"}});
    store
        .apply_events_page("acc", "cal", &[ev(e.clone())], None)
        .unwrap();
    store
        .apply_events_page("acc", "hid", &[ev(e)], None)
        .unwrap();
    assert!(
        store
            .occurrences(&week_of_19_oct(), Tz::UTC)
            .unwrap()
            .is_empty()
    );
}

fn series() -> serde_json::Value {
    json!({
        "id": "m", "summary": "1:1", "location": "Room 4", "colorId": "11",
        "start": {"dateTime": "2026-10-05T10:00:00+01:00", "timeZone": "Europe/London"},
        "end": {"dateTime": "2026-10-05T10:30:00+01:00", "timeZone": "Europe/London"},
        "recurrence": ["RRULE:FREQ=WEEKLY"]
    })
}

#[test]
fn series_expands_with_master_fields() {
    let mut store = store_with("acc", visible());
    store
        .apply_events_page("acc", "cal", &[ev(series())], None)
        .unwrap();
    let got = store
        .occurrences(&week_of_19_oct(), Tz::Europe__London)
        .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].instance_id, "m_20261019T090000Z");
    assert_eq!(got[0].event_id, "m");
    assert_eq!(got[0].location.as_deref(), Some("Room 4"));
    assert_eq!(got[0].color_id.as_deref(), Some("11"));
}

#[test]
fn modified_exception_uses_its_own_fields() {
    let mut store = store_with("acc", visible());
    let moved = json!({
        "id": "m_20261019T090000Z", "summary": "1:1 (moved)", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-19T10:00:00+01:00", "timeZone": "Europe/London"},
        "start": {"dateTime": "2026-10-20T15:00:00+01:00"}, "end": {"dateTime": "2026-10-20T15:30:00+01:00"}
    });
    store
        .apply_events_page("acc", "cal", &[ev(series()), ev(moved)], None)
        .unwrap();
    let got = store
        .occurrences(&week_of_19_oct(), Tz::Europe__London)
        .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].summary.as_deref(), Some("1:1 (moved)"));
    assert_eq!(got[0].event_id, "m_20261019T090000Z");
}

#[test]
fn cancelled_exception_hides_its_occurrence() {
    let mut store = store_with("acc", visible());
    let cancelled = json!({
        "id": "m_20261019T090000Z", "status": "cancelled", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-19T10:00:00+01:00", "timeZone": "Europe/London"}
    });
    store
        .apply_events_page("acc", "cal", &[ev(series()), ev(cancelled)], None)
        .unwrap();
    assert!(
        store
            .occurrences(&week_of_19_oct(), Tz::Europe__London)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn exception_moved_past_series_end_still_appears() {
    let mut store = store_with("acc", visible());
    let mut short = series();
    short["recurrence"] = json!(["RRULE:FREQ=WEEKLY;COUNT=2"]);
    let moved = json!({
        "id": "m_20261012T090000Z", "summary": "Late 1:1", "recurringEventId": "m",
        "originalStartTime": {"dateTime": "2026-10-12T10:00:00+01:00", "timeZone": "Europe/London"},
        "start": {"dateTime": "2026-11-20T10:00:00Z"}, "end": {"dateTime": "2026-11-20T10:30:00Z"}
    });
    store
        .apply_events_page("acc", "cal", &[ev(short), ev(moved)], None)
        .unwrap();
    let november = local_days(d(2026, 11, 1), d(2026, 12, 1), Tz::UTC);
    let got = store.occurrences(&november, Tz::UTC).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].summary.as_deref(), Some("Late 1:1"));
}

#[test]
fn unparseable_series_shows_once_with_a_flag() {
    let mut store = store_with("acc", visible());
    let mut broken = series();
    broken["recurrence"] = json!(["RRULE:FREQ=SOMETIMES"]);
    broken["start"] = json!({"dateTime": "2026-10-20T10:00:00+01:00", "timeZone": "Europe/London"});
    broken["end"] = json!({"dateTime": "2026-10-20T10:30:00+01:00", "timeZone": "Europe/London"});
    store
        .apply_events_page("acc", "cal", &[ev(broken)], None)
        .unwrap();
    let got = store
        .occurrences(&week_of_19_oct(), Tz::Europe__London)
        .unwrap();
    assert_eq!(got.len(), 1);
    assert!(got[0].recurrence_error);
}

fn all_day_on(date: &str) -> Item<EventFields> {
    let next = (NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap() + chrono::TimeDelta::days(1))
        .format("%Y-%m-%d")
        .to_string();
    ev(json!({"id": "ad", "summary": "All day", "start": {"date": date}, "end": {"date": next}}))
}

#[test]
fn all_day_event_respects_display_zone() {
    let mut store = store_with("acc", visible());
    store
        .apply_events_page("acc", "cal", &[all_day_on("2026-10-20")], None)
        .unwrap();
    for tz in [Tz::Pacific__Honolulu, Tz::Pacific__Tongatapu] {
        let on_day = store
            .occurrences(&local_days(d(2026, 10, 20), d(2026, 10, 21), tz), tz)
            .unwrap();
        let before = store
            .occurrences(&local_days(d(2026, 10, 19), d(2026, 10, 20), tz), tz)
            .unwrap();
        let after = store
            .occurrences(&local_days(d(2026, 10, 21), d(2026, 10, 22), tz), tz)
            .unwrap();
        assert_eq!(
            (on_day.len(), before.len(), after.len()),
            (1, 0, 0),
            "zone {tz}"
        );
    }
}

#[test]
fn all_day_event_on_skipped_midnight_lands_on_its_day() {
    let tz = Tz::America__Santiago;
    let mut store = store_with("acc", visible());
    store
        .apply_events_page("acc", "cal", &[all_day_on("2026-09-06")], None)
        .unwrap();
    assert_eq!(
        store
            .occurrences(&local_days(d(2026, 9, 6), d(2026, 9, 7), tz), tz)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .occurrences(&local_days(d(2026, 9, 5), d(2026, 9, 6), tz), tz)
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn same_event_in_two_accounts_appears_twice() {
    let mut store = store_with("work", visible());
    add(&mut store, "home", visible());
    let shared = json!({"id": "x", "iCalUID": "x@google.com", "summary": "Shared",
        "start": {"dateTime": "2026-10-20T09:00:00Z"}, "end": {"dateTime": "2026-10-20T10:00:00Z"}});
    store
        .apply_events_page("work", "cal", &[ev(shared.clone())], None)
        .unwrap();
    store
        .apply_events_page("home", "cal", &[ev(shared)], None)
        .unwrap();
    let got = store.occurrences(&week_of_19_oct(), Tz::UTC).unwrap();
    let accounts: Vec<_> = got.iter().map(|o| o.account_id.as_str()).collect();
    assert_eq!(got.len(), 2);
    assert!(accounts.contains(&"work") && accounts.contains(&"home"));
}
