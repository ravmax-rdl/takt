//! Reading and writing the dates, times and durations typed into the editor.
//! Pure: no GPUI types.

use chrono::{Datelike as _, NaiveDate, NaiveTime, TimeDelta, Timelike as _, Weekday};

/// "20 Oct 2026": what the editor shows, and one of the forms it reads.
pub fn format_date(date: NaiveDate) -> String {
    date.format("%-d %b %Y").to_string()
}

pub fn format_time(time: NaiveTime, use_24h: bool) -> String {
    if use_24h {
        time.format("%H:%M").to_string()
    } else {
        time.format("%-I:%M %p").to_string()
    }
}

fn month_number(name: &str) -> Option<u32> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let name = name.to_ascii_lowercase();
    (name.len() >= 3)
        .then(|| MONTHS.iter().position(|m| name.starts_with(m)))
        .flatten()
        .map(|ix| ix as u32 + 1)
}

fn weekday(name: &str) -> Option<Weekday> {
    let name = name.to_ascii_lowercase();
    let days = [
        ("mon", Weekday::Mon),
        ("tue", Weekday::Tue),
        ("wed", Weekday::Wed),
        ("thu", Weekday::Thu),
        ("fri", Weekday::Fri),
        ("sat", Weekday::Sat),
        ("sun", Weekday::Sun),
    ];
    (name.len() >= 3)
        .then(|| days.iter().find(|(short, _)| name.starts_with(short)))
        .flatten()
        .map(|(_, day)| *day)
}

/// Reads a date. Accepts `2026-10-20`, `20 Oct`, `20 October 2026`,
/// `Oct 20`, `Tue 20 Oct`, `today`, `tomorrow`, `+3` (days from today) and a
/// weekday name (the next such day, today included). A date without a year
/// is the next one from `today` on, looking back at most three months.
pub fn parse_date(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let text = text.trim().trim_end_matches(',');
    if text.is_empty() {
        return None;
    }
    if let Ok(date) = text.parse::<NaiveDate>() {
        return Some(date);
    }
    match text.to_ascii_lowercase().as_str() {
        "today" | "tod" => return Some(today),
        "tomorrow" | "tom" => return Some(today + TimeDelta::days(1)),
        _ => {}
    }
    if let Some(days) = text.strip_prefix('+') {
        return days
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|n| (0..=3660).contains(n))
            .map(|n| today + TimeDelta::days(n));
    }
    let mut words: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',' || c == '.')
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() == 1
        && let Some(day) = weekday(words[0])
    {
        let ahead = (7 + day.num_days_from_monday() - today.weekday().num_days_from_monday()) % 7;
        return Some(today + TimeDelta::days(ahead.into()));
    }
    // A leading weekday is decoration: "Tue 20 Oct".
    if words.len() > 2 && weekday(words[0]).is_some() && words[0].parse::<u32>().is_err() {
        words.remove(0);
    }
    let (day, month, year) = match words.as_slice() {
        [a, b] | [a, b, _] => {
            let year = words.get(2).map(|y| y.parse::<i32>().ok());
            let (day, month) = match (a.parse::<u32>(), month_number(b)) {
                (Ok(day), Some(month)) => (day, month),
                _ => (b.parse::<u32>().ok()?, month_number(a)?),
            };
            (day, month, year)
        }
        _ => return None,
    };
    match year {
        Some(year) => {
            let year = year?;
            let year = if (0..100).contains(&year) {
                year + 2000
            } else {
                year
            };
            NaiveDate::from_ymd_opt(year, month, day)
        }
        None => {
            let this_year = NaiveDate::from_ymd_opt(today.year(), month, day);
            match this_year {
                Some(date) if date >= today - TimeDelta::days(92) => Some(date),
                _ => NaiveDate::from_ymd_opt(today.year() + 1, month, day),
            }
        }
    }
}

/// Reads a time of day. Accepts `9`, `9:30`, `09.30`, `930`, `1730`, `9am`,
/// `9:30 pm`, `12a`, `noon` and `midnight`.
pub fn parse_time(text: &str) -> Option<NaiveTime> {
    let text = text.trim().to_ascii_lowercase();
    match text.as_str() {
        "noon" => return NaiveTime::from_hms_opt(12, 0, 0),
        "midnight" => return NaiveTime::from_hms_opt(0, 0, 0),
        _ => {}
    }
    let (digits, half) =
        if let Some(rest) = text.strip_suffix("am").or_else(|| text.strip_suffix('a')) {
            (rest.trim_end(), Some(false))
        } else if let Some(rest) = text.strip_suffix("pm").or_else(|| text.strip_suffix('p')) {
            (rest.trim_end(), Some(true))
        } else {
            (text.as_str(), None)
        };
    let (hour, minute) = match digits.split_once([':', '.']) {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None if digits.len() >= 3 && digits.len() <= 4 => {
            let (h, m) = digits.split_at(digits.len() - 2);
            (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?)
        }
        None => (digits.parse::<u32>().ok()?, 0),
    };
    let hour = match half {
        Some(_) if hour == 0 || hour > 12 => return None,
        Some(false) => hour % 12,
        Some(true) => hour % 12 + 12,
        None => hour,
    };
    NaiveTime::from_hms_opt(hour, minute, 0)
}

/// The next half hour after `now`, for a new event's start.
pub fn next_half_hour(now: NaiveTime) -> NaiveTime {
    let minutes = now.hour() * 60 + now.minute();
    let next = (minutes / 30 + 1) * 30;
    NaiveTime::from_hms_opt((next / 60).min(23), next % 60, 0).unwrap_or(now)
}

/// A reminder's lead time split into the largest unit that divides it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Minutes,
    Hours,
    Days,
    Weeks,
}

impl Unit {
    pub const ALL: [Unit; 4] = [Unit::Minutes, Unit::Hours, Unit::Days, Unit::Weeks];

    pub fn minutes(self) -> u32 {
        match self {
            Unit::Minutes => 1,
            Unit::Hours => 60,
            Unit::Days => 24 * 60,
            Unit::Weeks => 7 * 24 * 60,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Unit::Minutes => "minutes",
            Unit::Hours => "hours",
            Unit::Days => "days",
            Unit::Weeks => "weeks",
        }
    }

    pub fn next(self) -> Unit {
        match self {
            Unit::Minutes => Unit::Hours,
            Unit::Hours => Unit::Days,
            Unit::Days => Unit::Weeks,
            Unit::Weeks => Unit::Minutes,
        }
    }
}

/// Google accepts reminders up to four weeks ahead.
pub const MAX_REMINDER_MINUTES: u32 = 40_320;

pub fn split_minutes(minutes: u32) -> (u32, Unit) {
    Unit::ALL
        .into_iter()
        .rev()
        .find(|unit| minutes > 0 && minutes.is_multiple_of(unit.minutes()))
        .map_or((minutes, Unit::Minutes), |unit| {
            (minutes / unit.minutes(), unit)
        })
}

/// The minutes for an amount typed next to a unit. `None` when it is not a
/// number or is further ahead than Google allows.
pub fn join_minutes(amount: &str, unit: Unit) -> Option<u32> {
    amount
        .trim()
        .parse::<u32>()
        .ok()?
        .checked_mul(unit.minutes())
        .filter(|minutes| *minutes <= MAX_REMINDER_MINUTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    /// Wednesday 11 November 2026.
    fn today() -> NaiveDate {
        d(2026, 11, 11)
    }

    #[test]
    fn dates_are_read_in_the_forms_people_type() {
        let cases = [
            ("2026-10-20", d(2026, 10, 20)),
            ("20 Oct 2026", d(2026, 10, 20)),
            ("Tue 20 Oct 2026", d(2026, 10, 20)),
            ("Tue, 20 Oct 2026", d(2026, 10, 20)),
            ("20 october 26", d(2026, 10, 20)),
            ("Oct 20, 2026", d(2026, 10, 20)),
            ("today", today()),
            ("Tomorrow", d(2026, 11, 12)),
            ("+3", d(2026, 11, 14)),
            ("+0", today()),
            ("fri", d(2026, 11, 13)),
            ("Wednesday", today()),
            ("tue", d(2026, 11, 17)),
            (" 24 dec ", d(2026, 12, 24)),
            // Recent past stays in this year; further back means next year.
            ("1 Nov", d(2026, 11, 1)),
            ("3 Jan", d(2027, 1, 3)),
            ("Mar 5", d(2027, 3, 5)),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_date(text, today()), Some(expected), "{text}");
        }
    }

    #[test]
    fn what_the_editor_shows_reads_back() {
        for date in [d(2026, 1, 1), d(2026, 10, 20), d(2028, 2, 29)] {
            assert_eq!(parse_date(&format_date(date), today()), Some(date));
        }
        assert_eq!(format_date(d(2026, 10, 5)), "5 Oct 2026");
        for time in [t(0, 0), t(9, 5), t(12, 0), t(23, 59)] {
            for use_24h in [true, false] {
                assert_eq!(parse_time(&format_time(time, use_24h)), Some(time));
            }
        }
        assert_eq!(format_time(t(13, 5), false), "1:05 PM");
    }

    #[test]
    fn nonsense_dates_are_refused() {
        for text in [
            "",
            "soon",
            "32 Oct",
            "30 Feb 2026",
            "Oct",
            "20",
            "+x",
            "20 Oct 2026 extra",
        ] {
            assert_eq!(parse_date(text, today()), None, "{text}");
        }
    }

    #[test]
    fn times_are_read_in_the_forms_people_type() {
        let cases = [
            ("9", t(9, 0)),
            ("09:30", t(9, 30)),
            ("9.30", t(9, 30)),
            ("930", t(9, 30)),
            ("1730", t(17, 30)),
            ("17", t(17, 0)),
            ("9am", t(9, 0)),
            ("9 PM", t(21, 0)),
            ("9:30pm", t(21, 30)),
            ("12a", t(0, 0)),
            ("12pm", t(12, 0)),
            ("12:15 am", t(0, 15)),
            ("noon", t(12, 0)),
            ("midnight", t(0, 0)),
            ("0", t(0, 0)),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_time(text), Some(expected), "{text}");
        }
        for text in ["", "25", "9:60", "13pm", "0am", "nine", "12345", "9:3x"] {
            assert_eq!(parse_time(text), None, "{text}");
        }
    }

    #[test]
    fn new_events_start_on_the_next_half_hour() {
        assert_eq!(next_half_hour(t(9, 0)), t(9, 30));
        assert_eq!(next_half_hour(t(9, 29)), t(9, 30));
        assert_eq!(next_half_hour(t(9, 30)), t(10, 0));
        assert_eq!(next_half_hour(t(23, 10)), t(23, 30));
        assert_eq!(next_half_hour(t(23, 45)), t(23, 0));
    }

    #[test]
    fn reminder_minutes_split_into_the_largest_whole_unit() {
        assert_eq!(split_minutes(10), (10, Unit::Minutes));
        assert_eq!(split_minutes(90), (90, Unit::Minutes));
        assert_eq!(split_minutes(120), (2, Unit::Hours));
        assert_eq!(split_minutes(1440), (1, Unit::Days));
        assert_eq!(split_minutes(20160), (2, Unit::Weeks));
        assert_eq!(split_minutes(0), (0, Unit::Minutes));
        assert_eq!(join_minutes(" 2 ", Unit::Hours), Some(120));
        assert_eq!(join_minutes("4", Unit::Weeks), Some(40_320));
        assert_eq!(join_minutes("5", Unit::Weeks), None);
        assert_eq!(join_minutes("x", Unit::Minutes), None);
        assert_eq!(Unit::Weeks.next(), Unit::Minutes);
    }
}
