mod common;

use common::*;
use takt_recur::{
    Exception, MAX_OCCURRENCES, Master, RecurError, Source, When, expand, series_end,
};

// Mondays 5, 12, 19, 26 October 2026, 09:00-10:00 UTC.
fn weekly() -> Master {
    Master {
        id: "w".into(),
        start: at("2026-10-05T09:00:00Z"),
        end: at("2026-10-05T10:00:00Z"),
        tz: chrono_tz::UTC,
        recurrence: vec!["RRULE:FREQ=WEEKLY;COUNT=4".into()],
    }
}

fn october() -> takt_recur::Range {
    range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z")
}

fn moved(original: When, start: &str, end: &str) -> Exception {
    Exception {
        id: "w_20261012T090000Z".into(),
        original_start: original,
        cancelled: false,
        start: at(start),
        end: at(end),
    }
}

#[test]
fn cancelled_exception_hides_occurrence() {
    let original = at("2026-10-12T09:00:00Z");
    let cancelled = Exception {
        id: "w_20261012T090000Z".into(),
        original_start: original,
        cancelled: true,
        start: original,
        end: original,
    };
    let got = expand(&weekly(), &[cancelled], &october()).unwrap();
    let ids: Vec<_> = got.iter().map(|o| o.instance_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "w_20261005T090000Z",
            "w_20261019T090000Z",
            "w_20261026T090000Z"
        ]
    );
}

#[test]
fn modified_exception_replaces_occurrence() {
    let e = moved(
        at("2026-10-12T09:00:00Z"),
        "2026-10-13T15:00:00Z",
        "2026-10-13T16:00:00Z",
    );
    let got = expand(&weekly(), &[e], &october()).unwrap();
    assert_eq!(got.len(), 4);
    assert_eq!(got[1].start, at("2026-10-13T15:00:00Z"));
    assert_eq!(got[1].source, Source::Exception);
    assert_eq!(got[1].original_start, at("2026-10-12T09:00:00Z"));
    assert_eq!(got[1].instance_id, "w_20261012T090000Z");
}

#[test]
fn exception_moved_into_range_from_outside_is_included() {
    let e = moved(
        at("2026-10-12T09:00:00Z"),
        "2026-10-20T12:00:00Z",
        "2026-10-20T13:00:00Z",
    );
    let got = expand(
        &weekly(),
        &[e],
        &range("2026-10-20T00:00:00Z", "2026-10-21T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].start, at("2026-10-20T12:00:00Z"));
}

#[test]
fn exception_moved_out_of_range_hides_its_original() {
    let e = moved(
        at("2026-10-12T09:00:00Z"),
        "2026-10-30T09:00:00Z",
        "2026-10-30T10:00:00Z",
    );
    let got = expand(
        &weekly(),
        &[e],
        &range("2026-10-12T00:00:00Z", "2026-10-13T00:00:00Z"),
    )
    .unwrap();
    assert!(got.is_empty());
}

#[test]
fn unparseable_rule_is_an_error() {
    let mut m = weekly();
    m.recurrence = vec!["RRULE:FREQ=SOMETIMES".into()];
    assert!(matches!(
        expand(&m, &[], &october()),
        Err(RecurError::Parse(_))
    ));
}

#[test]
fn start_and_end_kinds_must_match() {
    let mut m = weekly();
    m.end = day("2026-10-06");
    assert_eq!(expand(&m, &[], &october()), Err(RecurError::MixedStartEnd));
}

#[test]
fn expansion_stops_at_cap() {
    let m = Master {
        id: "d".into(),
        start: at("2000-01-01T09:00:00Z"),
        end: at("2000-01-01T09:30:00Z"),
        tz: chrono_tz::UTC,
        recurrence: vec!["RRULE:FREQ=DAILY".into()],
    };
    let got = expand(
        &m,
        &[],
        &range("2000-01-01T00:00:00Z", "2100-01-01T00:00:00Z"),
    )
    .unwrap();
    assert_eq!(got.len(), MAX_OCCURRENCES as usize);
}

#[test]
fn series_end_is_none_without_until_or_count() {
    let mut m = weekly();
    m.recurrence = vec!["RRULE:FREQ=WEEKLY".into()];
    assert_eq!(series_end(&m), Ok(None));
}

#[test]
fn series_end_for_count() {
    assert_eq!(series_end(&weekly()), Ok(Some(at("2026-10-26T10:00:00Z"))));
}

#[test]
fn series_end_for_all_day_until() {
    let m = Master {
        id: "a".into(),
        start: day("2026-10-05"),
        end: day("2026-10-06"),
        tz: chrono_tz::UTC,
        recurrence: vec!["RRULE:FREQ=WEEKLY;UNTIL=20261019".into()],
    };
    assert_eq!(series_end(&m), Ok(Some(day("2026-10-20"))));
}
