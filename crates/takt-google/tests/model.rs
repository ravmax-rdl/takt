use chrono::NaiveDate;
use serde_json::json;
use takt_google::GoogleError;
use takt_google::model::*;

#[test]
fn event_keeps_unknown_fields_in_raw() {
    let raw = json!({
        "kind": "calendar#event", "id": "e1", "status": "confirmed", "summary": "Standup",
        "iCalUID": "e1@google.com",
        "start": {"dateTime": "2026-10-17T09:00:00+01:00", "timeZone": "Europe/London"},
        "end": {"dateTime": "2026-10-17T09:30:00+01:00", "timeZone": "Europe/London"},
        "conferenceData": {"entryPoints": [{"uri": "https://meet.google.com/abc"}]},
        "x-future-field": {"nested": [1, 2, 3]}
    });
    let item = Item::<EventFields>::from_value(raw.clone()).unwrap();
    assert_eq!(item.raw, raw);
    assert_eq!(item.fields.id, "e1");
    assert_eq!(item.fields.ical_uid.as_deref(), Some("e1@google.com"));
    let start = item.fields.start.as_ref().unwrap();
    assert_eq!(start.time_zone.as_deref(), Some("Europe/London"));
    assert_eq!(
        start.date_time.unwrap().to_rfc3339(),
        "2026-10-17T09:00:00+01:00"
    );
}

#[test]
fn cancelled_instance_with_minimal_fields_parses() {
    let item = Item::<EventFields>::from_value(json!({
        "id": "e1_20261017T080000Z", "status": "cancelled", "recurringEventId": "e1",
        "originalStartTime": {"dateTime": "2026-10-17T09:00:00+01:00", "timeZone": "Europe/London"}
    }))
    .unwrap();
    assert!(item.fields.is_cancelled());
    assert_eq!(item.fields.recurring_event_id.as_deref(), Some("e1"));
    assert!(item.fields.start.is_none());
    assert!(item.fields.original_start_time.is_some());
}

#[test]
fn all_day_event_reads_date() {
    let item = Item::<EventFields>::from_value(json!({
        "id": "a", "start": {"date": "2026-10-20"}, "end": {"date": "2026-10-21"}
    }))
    .unwrap();
    assert_eq!(
        item.fields.start.unwrap().date,
        NaiveDate::from_ymd_opt(2026, 10, 20)
    );
}

#[test]
fn self_response_comes_from_self_attendee() {
    let item = Item::<EventFields>::from_value(json!({
        "id": "m",
        "attendees": [
            {"email": "a@example.com", "responseStatus": "accepted"},
            {"email": "me@example.com", "self": true, "responseStatus": "declined"}
        ]
    }))
    .unwrap();
    assert_eq!(item.fields.self_response(), Some("declined"));
}

#[test]
fn calendar_list_entry_flags_default_to_false() {
    let item = Item::<CalendarListEntry>::from_value(
        json!({"id": "me@example.com", "summary": "Me", "accessRole": "owner"}),
    )
    .unwrap();
    let c = item.fields;
    assert!(!c.selected && !c.hidden && !c.primary && !c.deleted);
    assert_eq!(c.display_name(), "Me");
}

#[test]
fn summary_override_wins_for_display_name() {
    let item = Item::<CalendarListEntry>::from_value(
        json!({"id": "x", "summary": "Team", "summaryOverride": "Work team"}),
    )
    .unwrap();
    assert_eq!(item.fields.display_name(), "Work team");
}

#[test]
fn task_due_date_is_date_part() {
    let item = Item::<TaskFields>::from_value(json!({
        "id": "t1", "title": "Pay rent", "status": "needsAction",
        "due": "2026-10-20T00:00:00.000Z", "updated": "2026-10-17T10:00:00.000Z"
    }))
    .unwrap();
    assert_eq!(
        item.fields.due_date(),
        NaiveDate::from_ymd_opt(2026, 10, 20)
    );
}

#[test]
fn colors_parse() {
    let colors: Colors = serde_json::from_value(json!({
        "calendar": {"1": {"background": "#ac725e", "foreground": "#1d1d1d"}},
        "event": {"1": {"background": "#a4bdfc", "foreground": "#1d1d1d"}}
    }))
    .unwrap();
    assert_eq!(colors.event["1"].background, "#a4bdfc");
}

#[test]
fn page_reads_tokens_and_items() {
    let page = Page::<EventFields>::from_value(json!({
        "items": [{"id": "a"}, {"id": "b"}], "nextPageToken": "p2"
    }))
    .unwrap();
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.next_page_token.as_deref(), Some("p2"));
    assert_eq!(page.next_sync_token, None);
}

#[test]
fn page_without_items_is_empty() {
    let page = Page::<EventFields>::from_value(json!({"nextSyncToken": "s1"})).unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.next_sync_token.as_deref(), Some("s1"));
}

#[test]
fn missing_id_is_a_decode_error() {
    let err = Item::<EventFields>::from_value(json!({"summary": "no id"})).unwrap_err();
    assert!(matches!(err, GoogleError::Decode(_)));
}
