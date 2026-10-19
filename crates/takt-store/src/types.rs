use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use takt_recur::{Range, When};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountStatus {
    Ok,
    NeedsReauth,
}

impl AccountStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            AccountStatus::Ok => "ok",
            AccountStatus::NeedsReauth => "needs_reauth",
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        if value == "needs_reauth" {
            AccountStatus::NeedsReauth
        } else {
            AccountStatus::Ok
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
    pub account_id: String,
    pub email: String,
    pub status: AccountStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarRow {
    pub account_id: String,
    pub calendar_id: String,
    pub summary: String,
    pub time_zone: Option<String>,
    pub background_color: Option<String>,
    pub foreground_color: Option<String>,
    pub access_role: Option<String>,
    pub selected: bool,
    pub hidden: bool,
    pub primary: bool,
}

/// A stretch of time. `end: None` runs forever (an endless series).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSpan {
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
}

impl TimeSpan {
    pub fn union(self, other: TimeSpan) -> TimeSpan {
        TimeSpan {
            start: self.start.min(other.start),
            end: match (self.end, other.end) {
                (Some(a), Some(b)) => Some(a.max(b)),
                _ => None,
            },
        }
    }

    pub fn overlaps(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        self.start < end && self.end.is_none_or(|e| e > start)
    }
}

/// What one write touched, so the UI knows whether to re-read its range.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub calendars: BTreeSet<String>,
    pub task_lists: BTreeSet<String>,
    /// Union of the times that changed.
    pub span: Option<TimeSpan>,
    /// Something changed whose times are unknown (e.g. a calendar's colour): re-read every range.
    pub everything: bool,
}

impl ChangeSet {
    pub fn is_empty(&self) -> bool {
        self.calendars.is_empty() && self.task_lists.is_empty() && !self.everything
    }

    pub fn affects(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> bool {
        self.everything || self.span.is_some_and(|s| s.overlaps(start, end))
    }

    pub fn merge(&mut self, other: ChangeSet) {
        self.calendars.extend(other.calendars);
        self.task_lists.extend(other.task_lists);
        self.everything |= other.everything;
        if let Some(span) = other.span {
            self.add_span(span);
        }
    }

    pub(crate) fn touch_calendar(&mut self, calendar_id: &str, span: Option<TimeSpan>) {
        self.calendars.insert(calendar_id.to_owned());
        match span {
            Some(span) => self.add_span(span),
            None => self.everything = true,
        }
    }

    pub(crate) fn touch_task_list(&mut self, list_id: &str, span: Option<TimeSpan>) {
        self.task_lists.insert(list_id.to_owned());
        match span {
            Some(span) => self.add_span(span),
            None => self.everything = true,
        }
    }

    fn add_span(&mut self, span: TimeSpan) {
        self.span = Some(self.span.map_or(span, |current| current.union(span)));
    }
}

/// Google's colour palette for one account: colour ID -> (background, foreground).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ColorPalette {
    pub calendar: BTreeMap<String, (String, String)>,
    pub event: BTreeMap<String, (String, String)>,
}

/// One occurrence ready for display. `event_id` names the row whose JSON
/// describes it: the series for generated occurrences, the exception otherwise.
#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub account_id: String,
    pub calendar_id: String,
    pub event_id: String,
    pub instance_id: String,
    pub start: When,
    pub end: When,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub color_id: Option<String>,
    pub self_response: Option<String>,
    pub recurrence_error: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskItem {
    pub account_id: String,
    pub list_id: String,
    pub list_title: String,
    pub task_id: String,
    pub title: String,
    pub due_date: NaiveDate,
    pub completed: bool,
}

/// The UTC instant a local date begins in `tz`. When local midnight does not
/// exist (a DST jump at 00:00), the first valid local time that day.
pub fn day_start_utc(date: NaiveDate, tz: Tz) -> DateTime<Utc> {
    let midnight = date.and_time(NaiveTime::MIN);
    (0..=12)
        .map(|quarter| midnight + TimeDelta::minutes(15 * quarter))
        .find_map(|local| tz.from_local_datetime(&local).earliest())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| midnight.and_utc())
}

/// The range covering local dates `[from, to)` in `tz`.
pub fn local_days(from: NaiveDate, to: NaiveDate, tz: Tz) -> Range {
    Range {
        start: day_start_utc(from, tz),
        end: day_start_utc(to, tz),
    }
}

/// The instant a `When` begins as seen in `tz`. All-day dates start at local midnight.
pub fn display_instant(when: &When, tz: Tz) -> DateTime<Utc> {
    match when {
        When::At(t) => *t,
        When::Date(d) => day_start_utc(*d, tz),
    }
}
