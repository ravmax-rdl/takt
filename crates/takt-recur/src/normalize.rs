//! Rewrites Google's recurrence lines into the RFC 5545 text `rrule` accepts.

use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;

use crate::{Master, When};

/// A DTSTART line built from the series' first event, followed by its
/// recurrence lines with UNTIL, EXDATE and RDATE values made explicit.
pub(crate) fn rruleset_text(master: &Master, all_day: bool) -> String {
    let mut lines = vec![dtstart_line(master)];
    lines.extend(
        master
            .recurrence
            .iter()
            .map(|line| normalize_line(line.trim(), all_day, master.tz)),
    );
    lines.join("\n")
}

fn dtstart_line(master: &Master) -> String {
    match master.start {
        When::At(t) => format!(
            "DTSTART;TZID={}:{}",
            master.tz.name(),
            t.with_timezone(&master.tz).format("%Y%m%dT%H%M%S")
        ),
        // All-day series run in UTC so generated instants map straight back to dates.
        When::Date(d) => format!("DTSTART:{}T000000Z", d.format("%Y%m%d")),
    }
}

fn normalize_line(line: &str, all_day: bool, tz: Tz) -> String {
    let Some((head, value)) = line.split_once(':') else {
        return line.to_string();
    };
    let name = head.split(';').next().unwrap_or_default().to_ascii_uppercase();
    match name.as_str() {
        "RRULE" | "EXRULE" => format!("{name}:{}", normalize_until(value, all_day, tz)),
        "EXDATE" | "RDATE" => {
            let values: Vec<&str> = value.split(',').map(str::trim).collect();
            if values.iter().all(|v| is_date(v)) {
                let stamps: Vec<String> = values.iter().map(|v| format!("{v}T000000Z")).collect();
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
    } else if value.len() == 15 && !value.ends_with('Z') {
        if let Ok(local) = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S") {
            return if all_day { format!("{value}Z") } else { utc_stamp(local, tz) };
        }
    }
    value.to_string()
}

fn utc_stamp(local: NaiveDateTime, tz: Tz) -> String {
    let utc = tz
        .from_local_datetime(&local)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| local.and_utc());
    utc.format("%Y%m%dT%H%M%SZ").to_string()
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
            normalize_line("RRULE:FREQ=DAILY;UNTIL=20261012", false, chrono_tz::America::Los_Angeles),
            "RRULE:FREQ=DAILY;UNTIL=20261013T065959Z"
        );
    }

    #[test]
    fn floating_until_is_converted_from_series_zone() {
        assert_eq!(
            normalize_line("RRULE:FREQ=WEEKLY;UNTIL=20261012T090000", false, chrono_tz::Europe::London),
            "RRULE:FREQ=WEEKLY;UNTIL=20261012T080000Z"
        );
    }

    #[test]
    fn timezone_exdate_is_left_alone() {
        let line = "EXDATE;TZID=Europe/London:20261012T090000";
        assert_eq!(normalize_line(line, false, chrono_tz::Europe::London), line);
    }
}
