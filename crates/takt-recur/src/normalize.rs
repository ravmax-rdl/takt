//! Rewrites Google's recurrence lines into the RFC 5545 text `rrule` accepts.

use chrono::{DateTime, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;

use crate::{Master, RecurError, When};

/// The series' recurrence lines with UNTIL, EXDATE and RDATE values made
/// explicit. The start goes to `rrule` separately (see [`rule_start`]), so
/// any DTSTART or DTEND line is dropped rather than allowed to override it.
pub(crate) fn recurrence_text(master: &Master, all_day: bool) -> String {
    master
        .recurrence
        .iter()
        .filter(|line| !is_start_or_end(line))
        .map(|line| normalize_line(line.trim(), all_day, master.tz))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_start_or_end(line: &str) -> bool {
    let name = line.trim().split([':', ';']).next().unwrap_or_default();
    name.eq_ignore_ascii_case("DTSTART") || name.eq_ignore_ascii_case("DTEND")
}

/// The instant `rrule` starts the series from.
///
/// `rrule` rejects a DTSTART whose local time is ambiguous, and when handed
/// the later of the two instants it generates the earlier one and then drops
/// it as being before the start. So a start in the repeated hour of a
/// fall-back is given to `rrule` as the earlier instant, which `rrule`
/// generates (and counts) as the first occurrence. The caller maps that one
/// occurrence back to the master's real start.
///
/// `rrule` also drops every day whose local midnight is not a single valid
/// time. If such a day holds an ambiguous start, the first occurrence would be
/// lost silently, so that combination is an error.
pub(crate) fn rule_start(master: &Master) -> Result<DateTime<rrule::Tz>, RecurError> {
    Ok(match master.start {
        When::At(t) => {
            let local = t.with_timezone(&master.tz).naive_local();
            let start = match master.tz.from_local_datetime(&local) {
                LocalResult::Ambiguous(earlier, _) => {
                    let midnight = local.date().and_time(chrono::NaiveTime::MIN);
                    if !matches!(
                        master.tz.from_local_datetime(&midnight),
                        LocalResult::Single(_)
                    ) {
                        return Err(RecurError::Parse(format!(
                            "series start {local} is ambiguous in {} and local midnight that day is not a valid single time",
                            master.tz
                        )));
                    }
                    earlier.with_timezone(&Utc)
                }
                _ => t,
            };
            start.with_timezone(&rrule::Tz::Tz(master.tz))
        }
        // All-day series run in UTC so generated instants map straight back to dates.
        When::Date(d) => d
            .and_hms_opt(0, 0, 0)
            .expect("midnight exists")
            .and_utc()
            .with_timezone(&rrule::Tz::UTC),
    })
}

fn normalize_line(line: &str, all_day: bool, tz: Tz) -> String {
    let Some((head, value)) = line.split_once(':') else {
        return line.to_string();
    };
    let name = head
        .split(';')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    match name.as_str() {
        "RRULE" | "EXRULE" => format!("{name}:{}", normalize_until(value, all_day, tz)),
        "EXDATE" | "RDATE" => {
            let values: Vec<&str> = value.split(',').map(str::trim).collect();
            if values.iter().all(|v| is_date(v)) {
                let stamps: Vec<String> = values.iter().map(|v| format!("{v}T000000Z")).collect();
                format!("{name}:{}", stamps.join(","))
            } else if let Some(stamps) = unambiguous_stamps(head, &values) {
                format!("{name}:{}", stamps.join(","))
            } else {
                line.to_string()
            }
        }
        _ => line.to_string(),
    }
}

fn normalize_until(rule: &str, all_day: bool, tz: Tz) -> String {
    rule.split(';')
        .map(|part| match part.split_once('=') {
            Some((key, value)) if key.eq_ignore_ascii_case("UNTIL") => {
                format!("UNTIL={}", until_value(value, all_day, tz))
            }
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Google sometimes writes UNTIL as a bare date or a floating local time.
/// RFC 5545 wants UTC when DTSTART carries a zone, so convert.
fn until_value(value: &str, all_day: bool, tz: Tz) -> String {
    if is_date(value) {
        if all_day {
            return format!("{value}T000000Z");
        }
        if let Ok(d) = NaiveDate::parse_from_str(value, "%Y%m%d") {
            return utc_stamp(d.and_hms_opt(23, 59, 59).expect("valid time"), tz);
        }
    } else if value.len() == 15
        && !value.ends_with('Z')
        && let Ok(local) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
    {
        return if all_day {
            format!("{value}Z")
        } else {
            utc_stamp(local, tz)
        };
    }
    value.to_string()
}

/// `rrule` rejects TZID values that fall in a DST gap or overlap. When a
/// line has any, all its values are rewritten as UTC; other lines are left
/// alone.
fn unambiguous_stamps(head: &str, values: &[&str]) -> Option<Vec<String>> {
    let tz: Tz = head
        .split(';')
        .skip(1)
        .find_map(|param| {
            let (key, value) = param.split_once('=')?;
            key.eq_ignore_ascii_case("TZID").then_some(value)
        })?
        .parse()
        .ok()?;
    let locals = values
        .iter()
        .map(|v| NaiveDateTime::parse_from_str(v, "%Y%m%dT%H%M%S").ok())
        .collect::<Option<Vec<_>>>()?;
    locals
        .iter()
        .any(|local| !matches!(tz.from_local_datetime(local), LocalResult::Single(_)))
        .then(|| locals.iter().map(|local| utc_stamp(*local, tz)).collect())
}

fn utc_stamp(local: NaiveDateTime, tz: Tz) -> String {
    local_to_utc(local, tz).format("%Y%m%dT%H%M%SZ").to_string()
}

/// RFC 5545 3.3.5: a repeated local time means its first occurrence, and a
/// skipped one is read with the UTC offset in force before the gap.
fn local_to_utc(local: NaiveDateTime, tz: Tz) -> DateTime<Utc> {
    match tz.from_local_datetime(&local) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => t.with_timezone(&Utc),
        LocalResult::None => {
            // Gaps last at most a day and transitions are months apart, so the
            // offset two days earlier is the one before the gap.
            let before = tz
                .offset_from_utc_datetime(&(local - TimeDelta::days(2)))
                .fix();
            (local - before).and_utc()
        }
    }
}

fn is_date(value: &str) -> bool {
    value.len() == 8 && value.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_day_exdate_dates_become_utc_midnight() {
        assert_eq!(
            normalize_line("EXDATE;VALUE=DATE:20261003,20261005", true, Tz::UTC),
            "EXDATE:20261003T000000Z,20261005T000000Z"
        );
    }

    #[test]
    fn timed_date_only_until_is_end_of_local_day() {
        assert_eq!(
            normalize_line(
                "RRULE:FREQ=DAILY;UNTIL=20261012",
                false,
                chrono_tz::America::Los_Angeles
            ),
            "RRULE:FREQ=DAILY;UNTIL=20261013T065959Z"
        );
    }

    #[test]
    fn floating_until_is_converted_from_series_zone() {
        assert_eq!(
            normalize_line(
                "RRULE:FREQ=WEEKLY;UNTIL=20261012T090000",
                false,
                chrono_tz::Europe::London
            ),
            "RRULE:FREQ=WEEKLY;UNTIL=20261012T080000Z"
        );
    }

    #[test]
    fn repeated_local_exdate_becomes_utc_first_occurrence() {
        assert_eq!(
            normalize_line(
                "EXDATE;TZID=Europe/Berlin:20181027T020000,20181028T020000",
                false,
                chrono_tz::Europe::Berlin
            ),
            "EXDATE:20181027T000000Z,20181028T000000Z"
        );
    }

    #[test]
    fn skipped_local_rdate_uses_offset_before_gap() {
        assert_eq!(
            normalize_line(
                "RDATE;TZID=Europe/Berlin:20190331T023000",
                false,
                chrono_tz::Europe::Berlin
            ),
            "RDATE:20190331T013000Z"
        );
    }

    #[test]
    fn skipped_floating_until_uses_offset_before_gap() {
        assert_eq!(
            normalize_line(
                "RRULE:FREQ=DAILY;UNTIL=20190331T023000",
                false,
                chrono_tz::Europe::Berlin
            ),
            "RRULE:FREQ=DAILY;UNTIL=20190331T013000Z"
        );
    }

    #[test]
    fn dtstart_and_dtend_lines_are_dropped() {
        let master = Master {
            id: "m".into(),
            start: When::Date(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            end: When::Date(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap()),
            tz: Tz::UTC,
            recurrence: vec![
                "dtstart;TZID=UTC:20300101T000000".into(),
                " DTEND:20300102T000000Z".into(),
                "RRULE:FREQ=DAILY".into(),
            ],
        };
        assert_eq!(recurrence_text(&master, true), "RRULE:FREQ=DAILY");
    }

    #[test]
    fn timezone_exdate_is_left_alone() {
        let line = "EXDATE;TZID=Europe/London:20261012T090000";
        assert_eq!(normalize_line(line, false, chrono_tz::Europe::London), line);
    }
}
