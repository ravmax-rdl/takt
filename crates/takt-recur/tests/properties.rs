use std::collections::HashSet;

use chrono::{NaiveDate, TimeDelta, TimeZone, Utc};
use proptest::prelude::*;
use takt_recur::{Master, Range, Source, When, expand, series_end};

fn master(
    freq: &str,
    interval: u32,
    count: Option<u32>,
    start_day: i64,
    minute: i64,
    length_min: i64,
    all_day: bool,
) -> Master {
    let base = NaiveDate::from_ymd_opt(2000, 1, 1).unwrap() + TimeDelta::days(start_day);
    let mut rule = format!("RRULE:FREQ={freq};INTERVAL={interval}");
    if let Some(c) = count {
        rule.push_str(&format!(";COUNT={c}"));
    }
    let (start, end) = if all_day {
        let days = (length_min / 1440).max(1);
        (When::Date(base), When::Date(base + TimeDelta::days(days)))
    } else {
        let s =
            Utc.from_utc_datetime(&base.and_hms_opt(0, 0, 0).unwrap()) + TimeDelta::minutes(minute);
        (When::At(s), When::At(s + TimeDelta::minutes(length_min)))
    };
    Master {
        id: "p".into(),
        start,
        end,
        tz: chrono_tz::Europe::Berlin,
        recurrence: vec![rule],
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn expansion_invariants(
        freq in prop::sample::select(vec!["DAILY", "WEEKLY", "MONTHLY", "YEARLY"]),
        interval in 1u32..5,
        count in prop::option::of(1u32..60),
        start_day in 0i64..11_000,
        minute in 0i64..1440,
        length_min in 0i64..4_000,
        range_offset_days in prop_oneof![9 => 0i64..400, 1 => -400i64..0],
        range_len_days in 1i64..120,
        all_day in any::<bool>(),
    ) {
        let m = master(freq, interval, count, start_day, minute, length_min, all_day);
        let range_start = m.start.as_utc() + TimeDelta::days(range_offset_days);
        let range = Range { start: range_start, end: range_start + TimeDelta::days(range_len_days) };
        let got = expand(&m, &[], &range).expect("generated rules are valid");
        prop_assert!(got.windows(2).all(|w| w[0].start.as_utc() <= w[1].start.as_utc()));
        prop_assert!(got.iter().all(|o| range.overlaps(&o.start, &o.end)));
        let ids: HashSet<_> = got.iter().map(|o| o.instance_id.clone()).collect();
        prop_assert_eq!(ids.len(), got.len());
        prop_assert!(got.iter().all(|o| o.source == Source::Generated));
    }

    #[test]
    fn split_range_is_complete(
        freq in prop::sample::select(vec!["DAILY", "WEEKLY", "MONTHLY", "YEARLY"]),
        interval in 1u32..5,
        count in prop::option::of(1u32..60),
        start_day in 0i64..11_000,
        minute in 0i64..1440,
        length_min in 0i64..4_000,
        range_offset_days in prop_oneof![9 => 0i64..400, 1 => -400i64..0],
        first_len_days in 1i64..60,
        second_len_days in 1i64..60,
        all_day in any::<bool>(),
    ) {
        let m = master(freq, interval, count, start_day, minute, length_min, all_day);
        let a = m.start.as_utc() + TimeDelta::days(range_offset_days);
        let b = a + TimeDelta::days(first_len_days);
        let c = b + TimeDelta::days(second_len_days);
        let ids = |r: &Range| -> Vec<String> {
            expand(&m, &[], r)
                .expect("generated rules are valid")
                .into_iter()
                .map(|o| o.instance_id)
                .collect()
        };
        let whole = ids(&Range { start: a, end: c });
        let mut parts = ids(&Range { start: a, end: b });
        parts.extend(ids(&Range { start: b, end: c }));
        parts.sort();
        parts.dedup();
        let mut whole_sorted = whole.clone();
        whole_sorted.sort();
        prop_assert_eq!(whole_sorted, parts);
    }

    #[test]
    fn arbitrary_rule_text_never_panics(text in "\\PC{0,80}") {
        let mut m = master("DAILY", 1, Some(3), 9000, 540, 30, false);
        m.recurrence = vec![format!("RRULE:{text}")];
        let range = Range { start: m.start.as_utc(), end: m.start.as_utc() + TimeDelta::days(30) };
        let _ = expand(&m, &[], &range);
        let _ = series_end(&m);
    }
}
