use std::collections::{HashMap, HashSet};

use chrono::TimeDelta;
use chrono_tz::Tz;
use rusqlite::params;
use takt_google::model::EventFields;
use takt_recur::{Range, Source, When};

use crate::convert::{exception_of, master_of, when_of, zone};
use crate::{Occurrence, Store, StoreError, display_instant};

/// A visible `events` row with its calendar's zone.
struct Row {
    account_id: String,
    calendar_id: String,
    calendar_tz: Option<String>,
    fields: EventFields,
}

const VISIBLE: &str = "FROM events e JOIN calendars c
     ON c.account_id = e.account_id AND c.calendar_id = e.calendar_id
     WHERE c.selected = 1 AND c.hidden = 0";

impl Store {
    /// Everything visible in `range`, expanded and filtered exactly in `tz`.
    pub fn occurrences(&self, range: &Range, tz: Tz) -> Result<Vec<Occurrence>, StoreError> {
        // Index columns treat all-day dates as UTC midnight, so look a day wider
        // and filter precisely at the end.
        let padded = Range {
            start: range.start - TimeDelta::days(1),
            end: range.end + TimeDelta::days(1),
        };
        let (lo, hi) = (padded.start.timestamp(), padded.end.timestamp());
        let mut out = Vec::new();

        for row in self.rows(
            &format!("{VISIBLE} AND e.kind = 'single' AND e.start_utc < ?2 AND e.end_utc >= ?1"),
            lo,
            hi,
        )? {
            let f = &row.fields;
            let (Some(start), Some(end)) = (
                f.start.as_ref().and_then(when_of),
                f.end.as_ref().and_then(when_of),
            ) else {
                continue;
            };
            out.push(occurrence(&row, &f.id, f.id.clone(), start, end, f, false));
        }

        let mut expanded = HashSet::new();
        for row in self.rows(
            &format!(
                "{VISIBLE} AND e.kind = 'master' AND e.start_utc < ?2
                 AND (e.series_end_utc IS NULL OR e.series_end_utc >= ?1)"
            ),
            lo,
            hi,
        )? {
            expanded.insert((
                row.account_id.clone(),
                row.calendar_id.clone(),
                row.fields.id.clone(),
            ));
            self.expand_series(&row, &padded, &mut out)?;
        }

        // Exceptions moved somewhere their series does not reach in this range.
        for row in self.rows(
            &format!(
                "{VISIBLE} AND e.kind = 'exception' AND (e.status IS NULL OR e.status != 'cancelled')
                 AND e.start_utc < ?2 AND e.end_utc >= ?1"
            ),
            lo,
            hi,
        )? {
            let master_id = row.fields.recurring_event_id.clone().unwrap_or_default();
            if expanded.contains(&(row.account_id.clone(), row.calendar_id.clone(), master_id)) {
                continue;
            }
            if let Some(e) = exception_of(&row.fields) {
                out.push(occurrence(&row, &row.fields.id, e.id.clone(), e.start, e.end, &row.fields, false));
            }
        }

        out.retain(|o| overlaps_in(range, &o.start, &o.end, tz));
        out.sort_by(|a, b| {
            display_instant(&a.start, tz)
                .cmp(&display_instant(&b.start, tz))
                .then_with(|| a.summary.cmp(&b.summary))
                .then_with(|| a.account_id.cmp(&b.account_id))
        });
        Ok(out)
    }

    fn expand_series(
        &self,
        master: &Row,
        range: &Range,
        out: &mut Vec<Occurrence>,
    ) -> Result<(), StoreError> {
        let exception_rows =
            self.exceptions_of(&master.account_id, &master.calendar_id, &master.fields.id)?;
        let exceptions: Vec<_> = exception_rows.iter().filter_map(exception_of).collect();
        let by_id: HashMap<&str, &EventFields> =
            exception_rows.iter().map(|f| (f.id.as_str(), f)).collect();
        let fallback = zone(master.calendar_tz.as_deref()).unwrap_or(Tz::UTC);
        let expanded = master_of(&master.fields, fallback)
            .ok_or_else(|| takt_recur::RecurError::Parse("series has no start or end".into()))
            .and_then(|m| takt_recur::expand(&m, &exceptions, range));
        match expanded {
            Ok(list) => {
                for o in list {
                    let source = match o.source {
                        Source::Exception => by_id
                            .get(o.instance_id.as_str())
                            .copied()
                            .unwrap_or(&master.fields),
                        Source::Generated => &master.fields,
                    };
                    out.push(occurrence(
                        master,
                        &source.id,
                        o.instance_id,
                        o.start,
                        o.end,
                        source,
                        false,
                    ));
                }
            }
            Err(error) => {
                tracing::warn!(event = %master.fields.id, %error, "showing series as a single event");
                let f = &master.fields;
                if let (Some(start), Some(end)) = (
                    f.start.as_ref().and_then(when_of),
                    f.end.as_ref().and_then(when_of),
                ) {
                    out.push(occurrence(master, &f.id, f.id.clone(), start, end, f, true));
                }
            }
        }
        Ok(())
    }

    fn rows(&self, sql_tail: &str, lo: i64, hi: i64) -> Result<Vec<Row>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT e.account_id, e.calendar_id, c.time_zone, e.raw_json {sql_tail}"
        ))?;
        let raw_rows = stmt.query_map(params![lo, hi], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        let mut rows = Vec::new();
        for raw_row in raw_rows {
            let (account_id, calendar_id, calendar_tz, raw) = raw_row?;
            let fields: EventFields = serde_json::from_str(&raw)?;
            rows.push(Row {
                account_id,
                calendar_id,
                calendar_tz,
                fields,
            });
        }
        Ok(rows)
    }

    fn exceptions_of(
        &self,
        account_id: &str,
        calendar_id: &str,
        master_id: &str,
    ) -> Result<Vec<EventFields>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT raw_json FROM events WHERE account_id = ?1 AND calendar_id = ?2 AND recurring_event_id = ?3",
        )?;
        let raws = stmt.query_map(params![account_id, calendar_id, master_id], |row| {
            row.get::<_, String>(0)
        })?;
        let mut out = Vec::new();
        for raw in raws {
            out.push(serde_json::from_str(&raw?)?);
        }
        Ok(out)
    }
}

fn occurrence(
    row: &Row,
    event_id: &str,
    instance_id: String,
    start: When,
    end: When,
    fields: &EventFields,
    recurrence_error: bool,
) -> Occurrence {
    Occurrence {
        account_id: row.account_id.clone(),
        calendar_id: row.calendar_id.clone(),
        event_id: event_id.to_owned(),
        instance_id,
        start,
        end,
        summary: fields.summary.clone(),
        location: fields.location.clone(),
        color_id: fields.color_id.clone(),
        self_response: fields.self_response().map(str::to_owned),
        recurrence_error,
    }
}

fn overlaps_in(range: &Range, start: &When, end: &When, tz: Tz) -> bool {
    let (s, e) = (display_instant(start, tz), display_instant(end, tz));
    s < range.end && (e > range.start || (s == e && s >= range.start))
}
