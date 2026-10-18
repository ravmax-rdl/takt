//! Expands Google Calendar recurring series into occurrences.
//!
//! Pure functions: no I/O and no clock. All-day dates compare as UTC midnight
//! here. Callers that need display-time-zone precision pad the range by a day
//! and filter the result.

mod normalize;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};
use chrono_tz::Tz;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum When {
    At(DateTime<Utc>),
    Date(NaiveDate),
}

impl When {
    pub fn as_utc(&self) -> DateTime<Utc> {
        match self {
            When::At(t) => *t,
            When::Date(d) => d.and_time(NaiveTime::MIN).and_utc(),
        }
    }

    pub fn is_all_day(&self) -> bool {
        matches!(self, When::Date(_))
    }
}

/// Half-open interval `[start, end)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Range {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl Range {
    /// True when `[start, end)` intersects the range. A zero-length item counts
    /// when it sits inside the range.
    pub fn overlaps(&self, start: &When, end: &When) -> bool {
        let (s, e) = (start.as_utc(), end.as_utc());
        s < self.end && (e > self.start || (s == e && s >= self.start))
    }
}

/// The first event of a series, as Google stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct Master {
    pub id: String,
    pub start: When,
    pub end: When,
    /// IANA zone from `start.timeZone`. Unused for all-day series.
    pub tz: Tz,
    /// `recurrence` lines exactly as Google sent them.
    pub recurrence: Vec<String>,
}

/// A changed or cancelled occurrence of a series.
#[derive(Debug, Clone, PartialEq)]
pub struct Exception {
    pub id: String,
    pub original_start: When,
    pub cancelled: bool,
    pub start: When,
    pub end: When,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Generated,
    Exception,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Occurrence {
    pub instance_id: String,
    pub source: Source,
    pub original_start: When,
    pub start: When,
    pub end: When,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecurError {
    #[error("cannot parse recurrence: {0}")]
    Parse(String),
    #[error("series start and end disagree on all-day vs timed")]
    MixedStartEnd,
}

pub const MAX_OCCURRENCES: u16 = 10_000;

/// Google's ID for one occurrence of a series.
pub fn instance_id(master_id: &str, original_start: &When) -> String {
    match original_start {
        When::At(t) => format!("{master_id}_{}", t.format("%Y%m%dT%H%M%SZ")),
        When::Date(d) => format!("{master_id}_{}", d.format("%Y%m%d")),
    }
}

pub fn expand(
    master: &Master,
    exceptions: &[Exception],
    range: &Range,
) -> Result<Vec<Occurrence>, RecurError> {
    let all_day = series_kind(master)?;
    let duration = master.end.as_utc() - master.start.as_utc();
    let set = parse_set(master, all_day)?;
    let after = to_rrule_tz(range.start - duration - TimeDelta::seconds(1));
    let before = to_rrule_tz(range.end);
    let mut out: Vec<Occurrence> = set
        .after(after)
        .before(before)
        .all(MAX_OCCURRENCES)
        .dates
        .into_iter()
        .map(|dt| original_of(dt.with_timezone(&Utc), all_day))
        // Any exception, cancelled or moved, replaces the generated occurrence.
        .filter(|original| !exceptions.iter().any(|e| e.original_start == *original))
        .map(|original| Occurrence {
            instance_id: instance_id(&master.id, &original),
            source: Source::Generated,
            original_start: original,
            start: original,
            end: shift(&original, duration),
        })
        .filter(|o| range.overlaps(&o.start, &o.end))
        .collect();
    out.extend(
        exceptions
            .iter()
            .filter(|e| !e.cancelled && range.overlaps(&e.start, &e.end))
            .map(|e| Occurrence {
                instance_id: e.id.clone(),
                source: Source::Exception,
                original_start: e.original_start,
                start: e.start,
                end: e.end,
            }),
    );
    sort(&mut out);
    Ok(out)
}

/// End of the series' last occurrence, or `None` when it never ends (or has
/// more than [`MAX_OCCURRENCES`] occurrences, which callers treat the same way).
pub fn series_end(master: &Master) -> Result<Option<When>, RecurError> {
    let all_day = series_kind(master)?;
    let bounded = master
        .recurrence
        .iter()
        .map(|line| line.trim().to_ascii_uppercase())
        .filter(|line| line.starts_with("RRULE"))
        .all(|line| line.contains("UNTIL=") || line.contains("COUNT="));
    if !bounded {
        return Ok(None);
    }
    let duration = master.end.as_utc() - master.start.as_utc();
    let result = parse_set(master, all_day)?.all(MAX_OCCURRENCES);
    if result.limited {
        return Ok(None);
    }
    Ok(Some(match result.dates.last() {
        Some(dt) => shift(&original_of(dt.with_timezone(&Utc), all_day), duration),
        None => master.end,
    }))
}

fn series_kind(master: &Master) -> Result<bool, RecurError> {
    match (&master.start, &master.end) {
        (When::At(_), When::At(_)) => Ok(false),
        (When::Date(_), When::Date(_)) => Ok(true),
        _ => Err(RecurError::MixedStartEnd),
    }
}

fn parse_set(master: &Master, all_day: bool) -> Result<rrule::RRuleSet, RecurError> {
    normalize::rruleset_text(master, all_day)
        .parse::<rrule::RRuleSet>()
        .map_err(|e| RecurError::Parse(e.to_string()))
}

fn to_rrule_tz(t: DateTime<Utc>) -> DateTime<rrule::Tz> {
    t.with_timezone(&rrule::Tz::UTC)
}

fn original_of(t: DateTime<Utc>, all_day: bool) -> When {
    if all_day {
        When::Date(t.date_naive())
    } else {
        When::At(t)
    }
}

fn shift(start: &When, duration: TimeDelta) -> When {
    match start {
        When::At(t) => When::At(*t + duration),
        When::Date(d) => When::Date(*d + TimeDelta::days(duration.num_days())),
    }
}

fn sort(occurrences: &mut [Occurrence]) {
    occurrences.sort_by(|a, b| {
        a.start
            .as_utc()
            .cmp(&b.start.as_utc())
            .then_with(|| a.instance_id.cmp(&b.instance_id))
    });
}
