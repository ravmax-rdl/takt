use serde_json::json;
use takt_google::model::{CalendarListEntry, Colors, Item, Setting};
use takt_store::*;

fn cal(value: serde_json::Value) -> Item<CalendarListEntry> {
    Item::from_value(value).unwrap()
}

fn with_account() -> Store {
    let mut store = Store::open_in_memory().unwrap();
    store.upsert_account("acc", "me@example.com").unwrap();
    store
}

#[test]
fn fresh_store_has_no_accounts() {
    assert!(
        Store::open_in_memory()
            .unwrap()
            .accounts()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn on_disk_store_is_readable_from_a_read_only_connection() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("takt.sqlite3");
    let mut writer = Store::open(&path).unwrap();
    writer.upsert_account("acc", "me@example.com").unwrap();
    let reader = Store::open_read_only(&path).unwrap();
    assert_eq!(reader.accounts().unwrap().len(), 1);
}

#[test]
fn upsert_account_updates_email_and_status() {
    let mut store = with_account();
    store
        .set_account_status("acc", AccountStatus::NeedsReauth)
        .unwrap();
    assert_eq!(
        store.accounts().unwrap()[0].status,
        AccountStatus::NeedsReauth
    );
    store.upsert_account("acc", "renamed@example.com").unwrap();
    let accounts = store.accounts().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].email, "renamed@example.com");
    assert_eq!(accounts[0].status, AccountStatus::Ok);
}

#[test]
fn calendar_list_page_upserts_deletes_and_stores_token() {
    let mut store = with_account();
    let changes = store
        .apply_calendar_list_page(
            "acc",
            &[
                cal(json!({"id": "me@example.com", "summary": "Me", "primary": true, "selected": true, "backgroundColor": "#9fe1e7", "timeZone": "Europe/London"})),
                cal(json!({"id": "team@group.calendar.google.com", "summary": "Team", "summaryOverride": "Work team"})),
            ],
            Some("cl1"),
        )
        .unwrap();
    assert!(changes.everything);
    assert_eq!(
        store.calendar_list_sync_token("acc").unwrap().as_deref(),
        Some("cl1")
    );
    let calendars = store.calendars(Some("acc")).unwrap();
    assert_eq!(calendars.len(), 2);
    assert_eq!(
        calendars[0].calendar_id, "me@example.com",
        "primary sorts first"
    );
    assert!(calendars[0].selected && calendars[0].primary);
    assert_eq!(calendars[1].summary, "Work team");

    store
        .apply_calendar_list_page(
            "acc",
            &[cal(
                json!({"id": "team@group.calendar.google.com", "deleted": true}),
            )],
            Some("cl2"),
        )
        .unwrap();
    assert_eq!(store.calendars(Some("acc")).unwrap().len(), 1);
    assert_eq!(
        store.calendar_list_sync_token("acc").unwrap().as_deref(),
        Some("cl2")
    );
}

#[test]
fn settings_and_palette_round_trip() {
    let mut store = with_account();
    store
        .put_settings(
            "acc",
            &[Setting {
                id: "format24HourTime".into(),
                value: "false".into(),
            }],
        )
        .unwrap();
    assert_eq!(store.settings("acc").unwrap()["format24HourTime"], "false");
    let colors: Colors = serde_json::from_value(
        json!({"event": {"11": {"background": "#dc2127", "foreground": "#1d1d1d"}}}),
    )
    .unwrap();
    store.put_colors("acc", &colors).unwrap();
    assert_eq!(store.palette("acc").unwrap().event["11"].0, "#dc2127");
}

#[test]
fn removing_an_account_removes_its_calendars() {
    let mut store = with_account();
    store
        .apply_calendar_list_page("acc", &[cal(json!({"id": "c"}))], None)
        .unwrap();
    store.remove_account("acc").unwrap();
    assert!(store.accounts().unwrap().is_empty());
    assert!(store.calendars(None).unwrap().is_empty());
}

#[test]
fn reset_account_sync_clears_calendars_and_token() {
    let mut store = with_account();
    store
        .apply_calendar_list_page("acc", &[cal(json!({"id": "c"}))], Some("cl1"))
        .unwrap();
    store.reset_account_sync("acc").unwrap();
    assert!(store.calendars(Some("acc")).unwrap().is_empty());
    assert_eq!(store.calendar_list_sync_token("acc").unwrap(), None);
}

#[test]
fn day_start_utc_uses_local_midnight() {
    use chrono::{NaiveDate, TimeZone, Utc};
    let d = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    assert_eq!(
        day_start_utc(d, chrono_tz::Pacific::Honolulu),
        Utc.with_ymd_and_hms(2026, 10, 20, 10, 0, 0).unwrap()
    );
}

#[test]
fn day_start_utc_skips_missing_midnight() {
    use chrono::{NaiveDate, TimeZone, Utc};
    // Chile moves clocks from 00:00 to 01:00 on 6 September 2026.
    let d = NaiveDate::from_ymd_opt(2026, 9, 6).unwrap();
    assert_eq!(
        day_start_utc(d, chrono_tz::America::Santiago),
        Utc.with_ymd_and_hms(2026, 9, 6, 4, 0, 0).unwrap()
    );
}
