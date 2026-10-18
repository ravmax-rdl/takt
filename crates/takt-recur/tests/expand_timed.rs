mod common;

use common::*;
use takt_recur::{Master, Source, expand};

fn london_weekly() -> Master {
    Master {
        id: "abc".into(),
        start: at("2026-10-05T08:00:00Z"),
        end: at("2026-10-05T09:00:00Z"),
        tz: chrono_tz::Europe::London,
        recurrence: vec!["RRULE:FREQ=WEEKLY;COUNT=3".into()],
    }
}

#[test]
fn weekly_count_yields_google_instance_ids() {
    let got = expand(&london_weekly(), &[], &range("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z")).unwrap();
    let ids: Vec<_> = got.iter().map(|o| o.instance_id.as_str()).collect();
    assert_eq!(ids, ["abc_20261005T080000Z", "abc_20261012T080000Z", "abc_20261019T080000Z"]);
    assert!(got.iter().all(|o| o.source == Source::Generated));
    assert_eq!(got[1].end, at("2026-10-12T09:00:00Z"));
}

#[test]
fn occurrence_starting_before_range_but_overlapping_is_included() {
    let m = Master {
        id: "late".into(),
        start: at("2026-10-01T23:00:00Z"),
        end: at("2026-10-02T01:00:00Z"),
        tz: chrono_tz::UTC,
        recurrence: vec!["RRULE:FREQ=DAILY".into()],
    };
    let got = expand(&m, &[], &range("2026-10-10T00:00:00Z", "2026-10-11T00:00:00Z")).unwrap();
    let starts: Vec<_> = got.iter().map(|o| o.start).collect();
    assert_eq!(starts, [at("2026-10-09T23:00:00Z"), at("2026-10-10T23:00:00Z")]);
}

#[test]
fn wall_clock_time_is_kept_across_dst_change() {
    // 09:00 New York: EDT (UTC-4) on 26 Oct, EST (UTC-5) on 2 Nov 2026.
    let m = Master {
        id: "ny".into(),
        start: at("2026-10-26T13:00:00Z"),
        end: at("2026-10-26T13:30:00Z"),
        tz: chrono_tz::America::New_York,
        recurrence: vec!["RRULE:FREQ=WEEKLY;BYDAY=MO".into()],
    };
    let got = expand(&m, &[], &range("2026-10-26T00:00:00Z", "2026-11-03T00:00:00Z")).unwrap();
    let starts: Vec<_> = got.iter().map(|o| o.start).collect();
    assert_eq!(starts, [at("2026-10-26T13:00:00Z"), at("2026-11-02T14:00:00Z")]);
}

#[test]
fn twenty_year_old_daily_series_still_expands_today() {
    let m = Master {
        id: "old".into(),
        start: at("2005-01-01T09:00:00Z"),
        end: at("2005-01-01T09:15:00Z"),
        tz: chrono_tz::UTC,
        recurrence: vec!["RRULE:FREQ=DAILY".into()],
    };
    let got = expand(&m, &[], &range("2026-10-17T00:00:00Z", "2026-10-18T00:00:00Z")).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].instance_id, "old_20261017T090000Z");
}
