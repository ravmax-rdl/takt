mod common;

use common::*;
use takt_recur::{Master, expand};

fn all_day(id: &str, start: &str, end: &str, rules: &[&str]) -> Master {
    Master {
        id: id.into(),
        start: day(start),
        end: day(end),
        tz: chrono_tz::UTC,
        recurrence: rules.iter().map(|r| r.to_string()).collect(),
    }
}

fn ids(got: &[takt_recur::Occurrence]) -> Vec<&str> {
    got.iter().map(|o| o.instance_id.as_str()).collect()
}

#[test]
fn yearly_all_day_uses_date_instance_ids() {
    let m = all_day("bday", "2000-03-15", "2000-03-16", &["RRULE:FREQ=YEARLY"]);
    let got = expand(
        &m,
        &[],
        &range("2026-01-01T00:00:00Z", "2027-01-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(ids(&got), ["bday_20260315"]);
    assert_eq!(got[0].start, day("2026-03-15"));
    assert_eq!(got[0].end, day("2026-03-16"));
}

#[test]
fn exdate_value_date_removes_occurrence() {
    let m = all_day(
        "d",
        "2026-10-01",
        "2026-10-02",
        &["RRULE:FREQ=DAILY;COUNT=5", "EXDATE;VALUE=DATE:20261003"],
    );
    let got = expand(
        &m,
        &[],
        &range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(
        ids(&got),
        ["d_20261001", "d_20261002", "d_20261004", "d_20261005"]
    );
}

#[test]
fn date_only_until_is_inclusive_for_all_day() {
    let m = all_day(
        "w",
        "2026-10-05",
        "2026-10-06",
        &["RRULE:FREQ=WEEKLY;UNTIL=20261019"],
    );
    let got = expand(
        &m,
        &[],
        &range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(ids(&got), ["w_20261005", "w_20261012", "w_20261019"]);
}

#[test]
fn date_only_until_on_timed_series_reads_in_series_zone() {
    // 18:00 Los Angeles (PDT, UTC-7) daily until 12 Oct local: last start is 13 Oct 01:00 UTC.
    let m = Master {
        id: "la".into(),
        start: at("2026-10-10T01:00:00Z"),
        end: at("2026-10-10T02:00:00Z"),
        tz: chrono_tz::America::Los_Angeles,
        recurrence: vec!["RRULE:FREQ=DAILY;UNTIL=20261012".into()],
    };
    let got = expand(
        &m,
        &[],
        &range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(got.len(), 4);
    assert_eq!(got.last().unwrap().start, at("2026-10-13T01:00:00Z"));
}

#[test]
fn multi_day_all_day_keeps_its_length() {
    let m = all_day(
        "trip",
        "2026-10-01",
        "2026-10-04",
        &["RRULE:FREQ=WEEKLY;COUNT=2"],
    );
    let got = expand(
        &m,
        &[],
        &range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(got[1].start, day("2026-10-08"));
    assert_eq!(got[1].end, day("2026-10-11"));
}
