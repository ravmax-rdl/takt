//! Which days a view shows and how it moves. Pure: no GPUI types.

use chrono::{DateTime, Datelike, Months, NaiveDate, Offset as _, TimeDelta, Utc, Weekday};
use chrono_tz::Tz;

/// Days the agenda loads at a time.
pub const PAGE_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewKind {
    Day,
    #[default]
    Week,
    Month,
    Agenda,
}

impl ViewKind {
    pub const ALL: [ViewKind; 4] = [
        ViewKind::Day,
        ViewKind::Week,
        ViewKind::Month,
        ViewKind::Agenda,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ViewKind::Day => "Day",
            ViewKind::Week => "Week",
            ViewKind::Month => "Month",
            ViewKind::Agenda => "Agenda",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|v| v.label().eq_ignore_ascii_case(text.trim()))
    }

    /// Day and week draw a time grid with hours down the side.
    pub fn has_time_grid(self) -> bool {
        matches!(self, ViewKind::Day | ViewKind::Week)
    }
}

/// The first day of `date`'s week.
pub fn week_start_of(date: NaiveDate, week_start: Weekday) -> NaiveDate {
    let back = (7 + date.weekday().num_days_from_monday() - week_start.num_days_from_monday()) % 7;
    date - TimeDelta::days(i64::from(back))
}

fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

/// The days a view shows around `anchor`, as `from..to`. A month shows whole
/// weeks: four to six of them.
pub fn visible_days(
    view: ViewKind,
    anchor: NaiveDate,
    week_start: Weekday,
) -> (NaiveDate, NaiveDate) {
    match view {
        ViewKind::Day => (anchor, anchor + TimeDelta::days(1)),
        ViewKind::Week => {
            let from = week_start_of(anchor, week_start);
            (from, from + TimeDelta::days(7))
        }
        ViewKind::Month => {
            let first = month_start(anchor);
            let last = (first + Months::new(1)) - TimeDelta::days(1);
            (
                week_start_of(first, week_start),
                week_start_of(last, week_start) + TimeDelta::days(7),
            )
        }
        ViewKind::Agenda => (anchor, anchor + TimeDelta::days(PAGE_DAYS)),
    }
}

/// `anchor` moved by `count` of the view's periods. A month step keeps the
/// day of the month where the target month has it, else its last day.
pub fn step(view: ViewKind, anchor: NaiveDate, count: i64) -> NaiveDate {
    match view {
        ViewKind::Day => anchor + TimeDelta::days(count),
        ViewKind::Week => anchor + TimeDelta::days(7 * count),
        ViewKind::Month => {
            let months = Months::new(count.unsigned_abs() as u32);
            let moved = if count >= 0 {
                anchor.checked_add_months(months)
            } else {
                anchor.checked_sub_months(months)
            };
            moved.unwrap_or(anchor)
        }
        ViewKind::Agenda => anchor + TimeDelta::days(PAGE_DAYS * count),
    }
}

/// The toolbar title for the days `from..to` of a view anchored at `anchor`.
pub fn title(view: ViewKind, anchor: NaiveDate, from: NaiveDate, to: NaiveDate) -> String {
    match view {
        ViewKind::Day => anchor.format("%A %-d %B %Y").to_string(),
        ViewKind::Month => anchor.format("%B %Y").to_string(),
        ViewKind::Week | ViewKind::Agenda => {
            let last = to - TimeDelta::days(1);
            if from.year() != last.year() {
                format!(
                    "{} – {}",
                    from.format("%-d %b %Y"),
                    last.format("%-d %b %Y")
                )
            } else if from.month() != last.month() {
                format!("{} – {}", from.format("%-d %b"), last.format("%-d %b %Y"))
            } else {
                format!("{} – {}", from.format("%-d"), last.format("%-d %b %Y"))
            }
        }
    }
}

/// Google's `weekStart` setting: "0" Sunday, "1" Monday, "6" Saturday.
pub fn parse_week_start(value: &str) -> Option<Weekday> {
    match value.trim() {
        "0" => Some(Weekday::Sun),
        "1" => Some(Weekday::Mon),
        "6" => Some(Weekday::Sat),
        _ => None,
    }
}

/// A week start named in config.toml, e.g. "monday" or "sun".
pub fn parse_weekday(text: &str) -> Option<Weekday> {
    text.trim().parse().ok()
}

/// A zone's offset at `at`, the way Google labels it: "GMT+5:30", "GMT-3", "GMT".
pub fn zone_label(tz: Tz, at: DateTime<Utc>) -> String {
    let seconds = at.with_timezone(&tz).offset().fix().local_minus_utc();
    if seconds == 0 {
        return "GMT".into();
    }
    let sign = if seconds < 0 { '-' } else { '+' };
    let minutes = seconds.abs() / 60;
    if minutes % 60 == 0 {
        format!("GMT{sign}{}", minutes / 60)
    } else {
        format!("GMT{sign}{}:{:02}", minutes / 60, minutes % 60)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn week_starts_on_the_configured_day() {
        // 2026-10-17 is a Saturday.
        let sat = d(2026, 10, 17);
        assert_eq!(week_start_of(sat, Weekday::Mon), d(2026, 10, 12));
        assert_eq!(week_start_of(sat, Weekday::Sun), d(2026, 10, 11));
        assert_eq!(week_start_of(sat, Weekday::Sat), sat);
        assert_eq!(
            week_start_of(d(2026, 10, 12), Weekday::Mon),
            d(2026, 10, 12)
        );
    }

    #[test]
    fn visible_days_per_view() {
        let anchor = d(2026, 10, 17);
        assert_eq!(
            visible_days(ViewKind::Day, anchor, Weekday::Mon),
            (anchor, d(2026, 10, 18))
        );
        assert_eq!(
            visible_days(ViewKind::Week, anchor, Weekday::Mon),
            (d(2026, 10, 12), d(2026, 10, 19))
        );
        // October 2026: Thu 1st to Sat 31st, five Monday-weeks.
        assert_eq!(
            visible_days(ViewKind::Month, anchor, Weekday::Mon),
            (d(2026, 9, 28), d(2026, 11, 2))
        );
        assert_eq!(
            visible_days(ViewKind::Agenda, anchor, Weekday::Mon),
            (anchor, d(2026, 11, 16))
        );
    }

    #[test]
    fn a_month_shows_four_to_six_whole_weeks() {
        let weeks = |anchor, start| {
            let (from, to) = visible_days(ViewKind::Month, anchor, start);
            assert_eq!(from.weekday(), start);
            (to - from).num_days() / 7
        };
        // February 2026 starts on a Sunday and has 28 days.
        assert_eq!(weeks(d(2026, 2, 10), Weekday::Sun), 4);
        assert_eq!(weeks(d(2026, 2, 10), Weekday::Mon), 5);
        // August 2026 starts on a Saturday and has 31 days.
        assert_eq!(weeks(d(2026, 8, 10), Weekday::Mon), 6);
    }

    #[test]
    fn stepping_moves_one_period() {
        let anchor = d(2026, 1, 31);
        assert_eq!(step(ViewKind::Day, anchor, 1), d(2026, 2, 1));
        assert_eq!(step(ViewKind::Week, anchor, -1), d(2026, 1, 24));
        assert_eq!(step(ViewKind::Month, anchor, 1), d(2026, 2, 28));
        assert_eq!(step(ViewKind::Month, anchor, -2), d(2025, 11, 30));
        assert_eq!(step(ViewKind::Agenda, anchor, 1), d(2026, 3, 2));
    }

    #[test]
    fn titles() {
        let t = |view, anchor| {
            let (from, to) = visible_days(view, anchor, Weekday::Mon);
            title(view, anchor, from, to)
        };
        assert_eq!(
            t(ViewKind::Day, d(2026, 10, 17)),
            "Saturday 17 October 2026"
        );
        assert_eq!(t(ViewKind::Month, d(2026, 10, 17)), "October 2026");
        assert_eq!(t(ViewKind::Week, d(2026, 10, 17)), "12 – 18 Oct 2026");
        assert_eq!(t(ViewKind::Week, d(2026, 10, 1)), "28 Sep – 4 Oct 2026");
        assert_eq!(
            t(ViewKind::Week, d(2026, 12, 31)),
            "28 Dec 2026 – 3 Jan 2027"
        );
    }

    #[test]
    fn view_names_parse() {
        assert_eq!(ViewKind::parse(" week "), Some(ViewKind::Week));
        assert_eq!(ViewKind::parse("AGENDA"), Some(ViewKind::Agenda));
        assert_eq!(ViewKind::parse("year"), None);
    }

    #[test]
    fn week_start_settings_parse() {
        assert_eq!(parse_week_start("0"), Some(Weekday::Sun));
        assert_eq!(parse_week_start("6"), Some(Weekday::Sat));
        assert_eq!(parse_week_start("3"), None);
        assert_eq!(parse_weekday("monday"), Some(Weekday::Mon));
        assert_eq!(parse_weekday("Sun"), Some(Weekday::Sun));
        assert_eq!(parse_weekday("someday"), None);
    }

    #[test]
    fn zone_labels() {
        let winter = Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();
        let summer = Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap();
        assert_eq!(zone_label(Tz::UTC, winter), "GMT");
        assert_eq!(zone_label(Tz::Europe__Berlin, winter), "GMT+1");
        assert_eq!(zone_label(Tz::Europe__Berlin, summer), "GMT+2");
        assert_eq!(zone_label(Tz::Asia__Kolkata, winter), "GMT+5:30");
        assert_eq!(zone_label(Tz::America__St_Johns, winter), "GMT-3:30");
        assert_eq!(zone_label(Tz::America__New_York, summer), "GMT-4");
    }
}
