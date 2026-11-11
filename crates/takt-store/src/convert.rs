//! Turning Google's event fields into index columns and recurrence inputs.

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use chrono_tz::Tz;
use takt_google::model::{EventDateTime, EventFields};
use takt_recur::{Exception, Master, When};

use crate::TimeSpan;

const DAY: i64 = 86_400;

pub(crate) fn when_of(t: &EventDateTime) -> Option<When> {
    if let Some(date) = t.date {
        return Some(When::Date(date));
    }
    t.date_time.map(|dt| When::At(dt.with_timezone(&Utc)))
}

pub(crate) fn encode_when(when: &When) -> String {
    match when {
        When::At(t) => t.to_rfc3339_opts(SecondsFormat::Secs, true),
        When::Date(d) => d.format("%Y-%m-%d").to_string(),
    }
}

pub(crate) fn zone(name: Option<&str>) -> Option<Tz> {
    name.and_then(|n| n.parse().ok())
}

pub(crate) fn master_of(f: &EventFields, fallback: Tz) -> Option<Master> {
    let start = when_of(f.start.as_ref()?)?;
    let end = when_of(f.end.as_ref()?)?;
    let tz = zone(f.start.as_ref().and_then(|s| s.time_zone.as_deref())).unwrap_or(fallback);
    Some(Master {
        id: f.id.clone(),
        start,
        end,
        tz,
        recurrence: f.recurrence.clone(),
    })
}

pub(crate) fn exception_of(f: &EventFields) -> Option<Exception> {
    let original_start = when_of(f.original_start_time.as_ref()?)?;
    let start = f.start.as_ref().and_then(when_of).unwrap_or(original_start);
    let end = f.end.as_ref().and_then(when_of).unwrap_or(start);
    Some(Exception {
        id: f.id.clone(),
        original_start,
        cancelled: f.is_cancelled(),
        start,
        end,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Single,
    Master,
    Exception,
}

impl Kind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Kind::Single => "single",
            Kind::Master => "master",
            Kind::Exception => "exception",
        }
    }

    pub(crate) fn parse(text: &str) -> Kind {
        match text {
            "master" => Kind::Master,
            "exception" => Kind::Exception,
            _ => Kind::Single,
        }
    }
}

/// The index columns of one `events` row.
pub(crate) struct EventIndex {
    pub kind: Kind,
    pub start_utc: Option<i64>,
    pub end_utc: Option<i64>,
    pub all_day: bool,
    pub series_end_utc: Option<i64>,
    pub original_start: Option<String>,
}

impl EventIndex {
    pub(crate) fn of(f: &EventFields, fallback: Tz) -> EventIndex {
        if f.recurring_event_id.is_some() {
            let e = exception_of(f);
            return EventIndex {
                kind: Kind::Exception,
                start_utc: e.as_ref().map(|e| e.start.as_utc().timestamp()),
                end_utc: e.as_ref().map(|e| e.end.as_utc().timestamp()),
                all_day: e.as_ref().is_some_and(|e| e.start.is_all_day()),
                series_end_utc: None,
                original_start: e.as_ref().map(|e| encode_when(&e.original_start)),
            };
        }
        let start = f.start.as_ref().and_then(when_of);
        let end = f.end.as_ref().and_then(when_of).or(start);
        let all_day = start.is_some_and(|s| s.is_all_day());
        let start_utc = start.map(|s| s.as_utc().timestamp());
        let end_utc = end.map(|e| e.as_utc().timestamp());
        if f.recurrence.is_empty() {
            return EventIndex {
                kind: Kind::Single,
                start_utc,
                end_utc,
                all_day,
                series_end_utc: None,
                original_start: None,
            };
        }
        let series_end_utc = match master_of(f, fallback).map(|m| takt_recur::series_end(&m)) {
            Some(Ok(Some(last_end))) => Some(last_end.as_utc().timestamp()),
            Some(Ok(None)) => None,
            // Unparseable series: index it like a single event; queries flag it.
            Some(Err(_)) | None => end_utc,
        };
        EventIndex {
            kind: Kind::Master,
            start_utc,
            end_utc,
            all_day,
            series_end_utc,
            original_start: None,
        }
    }

    pub(crate) fn span(&self) -> Option<TimeSpan> {
        span_of(
            self.kind,
            self.start_utc,
            self.end_utc,
            self.all_day,
            self.series_end_utc,
        )
    }
}

/// The time a row covers. All-day rows are widened by a day on each side
/// because their real instants depend on the display time zone.
pub(crate) fn span_of(
    kind: Kind,
    start_utc: Option<i64>,
    end_utc: Option<i64>,
    all_day: bool,
    series_end_utc: Option<i64>,
) -> Option<TimeSpan> {
    let start = start_utc?;
    let end = match kind {
        Kind::Master => series_end_utc,
        _ => Some(end_utc.unwrap_or(start)),
    };
    let pad = if all_day { DAY } else { 0 };
    Some(TimeSpan {
        start: instant(start - pad),
        end: end.map(|e| instant(e + pad)),
    })
}

fn instant(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(seconds, 0)
        .single()
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}
