//! Series around DST changes in Europe/Berlin. On 2018-10-28 local
//! 02:00-02:59 happens twice (CEST then CET); on 2019-03-31 it never happens.

mod common;

use common::*;
use takt_recur::{Exception, Master, expand, series_end};

fn berlin(start: &str, end: &str, rules: &[&str]) -> Master {
    Master {
        id: "dst".into(),
        start: at(start),
        end: at(end),
        tz: chrono_tz::Europe::Berlin,
        recurrence: rules.iter().map(|r| r.to_string()).collect(),
    }
}

fn starts(m: &Master, exceptions: &[Exception], from: &str, to: &str) -> Vec<String> {
    expand(m, exceptions, &range(from, to))
        .unwrap()
        .into_iter()
        .map(|o| o.start.as_utc().to_rfc3339())
        .collect()
}

#[test]
fn start_in_first_of_repeated_hours() {
    // 02:00 CEST.
    let m = berlin(
        "2018-10-28T00:00:00Z",
        "2018-10-28T00:30:00Z",
        &["RRULE:FREQ=DAILY;COUNT=3"],
    );
    assert_eq!(
        starts(&m, &[], "2018-10-27T00:00:00Z", "2018-11-01T00:00:00Z"),
        [
            "2018-10-28T00:00:00+00:00",
            "2018-10-29T01:00:00+00:00",
            "2018-10-30T01:00:00+00:00",
        ]
    );
    assert_eq!(series_end(&m).unwrap(), Some(at("2018-10-30T01:30:00Z")));
}

#[test]
fn start_in_second_of_repeated_hours() {
    // 02:00 CET, the second 02:00 that night.
    let m = berlin(
        "2018-10-28T01:00:00Z",
        "2018-10-28T01:30:00Z",
        &["RRULE:FREQ=DAILY;COUNT=3"],
    );
    let got = expand(
        &m,
        &[],
        &range("2018-10-27T00:00:00Z", "2018-11-01T00:00:00Z"),
    )
    .unwrap();
    let ids: Vec<_> = got.iter().map(|o| o.instance_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "dst_20181028T010000Z",
            "dst_20181029T010000Z",
            "dst_20181030T010000Z",
        ]
    );
    assert_eq!(got[0].end, at("2018-10-28T01:30:00Z"));
    assert_eq!(series_end(&m).unwrap(), Some(at("2018-10-30T01:30:00Z")));
}

#[test]
fn second_of_repeated_hours_is_found_by_a_range_starting_at_it() {
    let m = berlin(
        "2018-10-28T01:00:00Z",
        "2018-10-28T01:00:00Z",
        &["RRULE:FREQ=DAILY"],
    );
    assert_eq!(
        starts(&m, &[], "2018-10-28T01:00:00Z", "2018-10-28T02:00:00Z"),
        ["2018-10-28T01:00:00+00:00"]
    );
}

#[test]
fn single_occurrence_in_second_of_repeated_hours_ends_with_master() {
    let m = berlin(
        "2018-10-28T01:15:00Z",
        "2018-10-28T02:00:00Z",
        &["RRULE:FREQ=WEEKLY;COUNT=1"],
    );
    assert_eq!(series_end(&m).unwrap(), Some(at("2018-10-28T02:00:00Z")));
}

#[test]
fn exception_on_second_of_repeated_hours_replaces_master() {
    let m = berlin(
        "2018-10-28T01:00:00Z",
        "2018-10-28T01:30:00Z",
        &["RRULE:FREQ=DAILY;COUNT=2"],
    );
    let moved = Exception {
        id: "dst_20181028T010000Z".into(),
        original_start: at("2018-10-28T01:00:00Z"),
        cancelled: true,
        start: at("2018-10-28T01:00:00Z"),
        end: at("2018-10-28T01:30:00Z"),
    };
    assert_eq!(
        starts(&m, &[moved], "2018-10-27T00:00:00Z", "2018-11-01T00:00:00Z"),
        ["2018-10-29T01:00:00+00:00"]
    );
}

#[test]
fn utc_exdate_on_second_of_repeated_hours_removes_master() {
    let m = berlin(
        "2018-10-28T01:00:00Z",
        "2018-10-28T01:30:00Z",
        &["RRULE:FREQ=DAILY;COUNT=2", "EXDATE:20181028T010000Z"],
    );
    assert_eq!(
        starts(&m, &[], "2018-10-27T00:00:00Z", "2018-11-01T00:00:00Z"),
        ["2018-10-29T01:00:00+00:00"]
    );
}

#[test]
fn later_occurrence_in_repeated_hour_keeps_wall_clock_at_first_offset() {
    // 02:30 CEST every day; on the fall-back day 02:30 means the first one.
    let m = berlin(
        "2018-10-26T00:30:00Z",
        "2018-10-26T01:30:00Z",
        &["RRULE:FREQ=DAILY;COUNT=4"],
    );
    assert_eq!(
        starts(&m, &[], "2018-10-25T00:00:00Z", "2018-11-01T00:00:00Z"),
        [
            "2018-10-26T00:30:00+00:00",
            "2018-10-27T00:30:00+00:00",
            "2018-10-28T00:30:00+00:00",
            "2018-10-29T01:30:00+00:00",
        ]
    );
}

#[test]
fn local_exdate_in_repeated_hour_is_accepted() {
    let m = berlin(
        "2018-10-26T00:30:00Z",
        "2018-10-26T01:30:00Z",
        &[
            "RRULE:FREQ=DAILY;COUNT=4",
            "EXDATE;TZID=Europe/Berlin:20181027T023000,20181028T023000",
        ],
    );
    assert_eq!(
        starts(&m, &[], "2018-10-25T00:00:00Z", "2018-11-01T00:00:00Z"),
        ["2018-10-26T00:30:00+00:00", "2018-10-29T01:30:00+00:00"]
    );
}

#[test]
fn local_rdate_in_repeated_hour_is_accepted() {
    let m = berlin(
        "2018-10-26T08:00:00Z",
        "2018-10-26T09:00:00Z",
        &[
            "RRULE:FREQ=WEEKLY;COUNT=1",
            "RDATE;TZID=Europe/Berlin:20181028T021500",
        ],
    );
    assert_eq!(
        starts(&m, &[], "2018-10-25T00:00:00Z", "2018-11-01T00:00:00Z"),
        ["2018-10-26T08:00:00+00:00", "2018-10-28T00:15:00+00:00"]
    );
}

#[test]
fn local_exdate_in_skipped_hour_matches_generated_occurrence() {
    // 02:30 CET every day; 2019-03-31 02:30 does not exist and runs at 03:30 CEST.
    let m = berlin(
        "2019-03-30T01:30:00Z",
        "2019-03-30T02:00:00Z",
        &[
            "RRULE:FREQ=DAILY;COUNT=3",
            "EXDATE;TZID=Europe/Berlin:20190331T023000",
        ],
    );
    assert_eq!(
        starts(&m, &[], "2019-03-29T00:00:00Z", "2019-04-05T00:00:00Z"),
        ["2019-03-30T01:30:00+00:00", "2019-04-01T00:30:00+00:00"]
    );
}

#[test]
fn floating_until_in_repeated_hour_includes_first_offset() {
    let m = berlin(
        "2018-10-26T00:30:00Z",
        "2018-10-26T01:00:00Z",
        &["RRULE:FREQ=DAILY;UNTIL=20181028T023000"],
    );
    assert_eq!(series_end(&m).unwrap(), Some(at("2018-10-28T01:00:00Z")));
}
