use chrono_tz::Tz;
use rusqlite::{OptionalExtension, Transaction, params};
use takt_google::model::{EventFields, Item};

use crate::convert::{EventIndex, Kind, span_of, zone};
use crate::{ChangeSet, Store, StoreError, TimeSpan};

const UPSERT_EVENT: &str = "
INSERT INTO events (account_id, calendar_id, event_id, raw_json, kind, status, ical_uid, recurring_event_id,
                    original_start, start_utc, end_utc, all_day, series_end_utc, etag, updated)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
ON CONFLICT (account_id, calendar_id, event_id) DO UPDATE SET
    raw_json = excluded.raw_json, kind = excluded.kind, status = excluded.status, ical_uid = excluded.ical_uid,
    recurring_event_id = excluded.recurring_event_id, original_start = excluded.original_start,
    start_utc = excluded.start_utc, end_utc = excluded.end_utc, all_day = excluded.all_day,
    series_end_utc = excluded.series_end_utc, etag = excluded.etag, updated = excluded.updated";

impl Store {
    pub fn events_sync_token(
        &self,
        account_id: &str,
        calendar_id: &str,
    ) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT events_sync_token FROM calendars WHERE account_id = ?1 AND calendar_id = ?2",
                params![account_id, calendar_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Applies one page of `events.list` in a single transaction. The sync
    /// token is written only when the page carries one (the last page).
    pub fn apply_events_page(
        &mut self,
        account_id: &str,
        calendar_id: &str,
        items: &[Item<EventFields>],
        next_sync_token: Option<&str>,
    ) -> Result<ChangeSet, StoreError> {
        let tx = self.conn.transaction()?;
        let fallback = calendar_zone(&tx, account_id, calendar_id)?;
        let mut changes = ChangeSet::default();
        for item in items {
            let f = &item.fields;
            if let Some(old) = stored_span(&tx, account_id, calendar_id, &f.id)? {
                changes.touch_calendar(calendar_id, Some(old));
            }
            if f.is_cancelled() && f.recurring_event_id.is_none() {
                // A cancelled series may arrive as only {id, status}; remove its exceptions too.
                tx.execute(
                    "DELETE FROM events WHERE account_id = ?1 AND calendar_id = ?2
                       AND (event_id = ?3 OR recurring_event_id = ?3)",
                    params![account_id, calendar_id, f.id],
                )?;
                continue;
            }
            let index = EventIndex::of(f, fallback);
            if let Some(span) = index.span() {
                changes.touch_calendar(calendar_id, Some(span));
            }
            tx.execute(
                UPSERT_EVENT,
                params![
                    account_id,
                    calendar_id,
                    f.id,
                    item.raw.to_string(),
                    index.kind.as_str(),
                    f.status,
                    f.ical_uid,
                    f.recurring_event_id,
                    index.original_start,
                    index.start_utc,
                    index.end_utc,
                    index.all_day,
                    index.series_end_utc,
                    f.etag,
                    f.updated.map(|u| u.to_rfc3339()),
                ],
            )?;
        }
        if let Some(token) = next_sync_token {
            tx.execute(
                "UPDATE calendars SET events_sync_token = ?3 WHERE account_id = ?1 AND calendar_id = ?2",
                params![account_id, calendar_id, token],
            )?;
        }
        tx.commit()?;
        Ok(changes)
    }

    /// After a 410: forget the calendar's events and token so it fully re-syncs.
    pub fn reset_calendar_events(
        &mut self,
        account_id: &str,
        calendar_id: &str,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM events WHERE account_id = ?1 AND calendar_id = ?2",
            params![account_id, calendar_id],
        )?;
        tx.execute(
            "UPDATE calendars SET events_sync_token = NULL WHERE account_id = ?1 AND calendar_id = ?2",
            params![account_id, calendar_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn event_raw(
        &self,
        account_id: &str,
        calendar_id: &str,
        event_id: &str,
    ) -> Result<Option<serde_json::Value>, StoreError> {
        let raw: Option<String> = self
            .conn
            .query_row(
                "SELECT raw_json FROM events WHERE account_id = ?1 AND calendar_id = ?2 AND event_id = ?3",
                params![account_id, calendar_id, event_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(raw.map(|text| serde_json::from_str(&text)).transpose()?)
    }

    pub fn event_ids(
        &self,
        account_id: &str,
        calendar_id: &str,
    ) -> Result<Vec<String>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT event_id FROM events WHERE account_id = ?1 AND calendar_id = ?2 ORDER BY event_id",
        )?;
        let rows = stmt.query_map(params![account_id, calendar_id], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

pub(crate) fn calendar_zone(
    tx: &Transaction<'_>,
    account_id: &str,
    calendar_id: &str,
) -> Result<Tz, StoreError> {
    let name: Option<String> = tx
        .query_row(
            "SELECT time_zone FROM calendars WHERE account_id = ?1 AND calendar_id = ?2",
            params![account_id, calendar_id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    Ok(zone(name.as_deref()).unwrap_or(Tz::UTC))
}

fn stored_span(
    tx: &Transaction<'_>,
    account_id: &str,
    calendar_id: &str,
    event_id: &str,
) -> Result<Option<TimeSpan>, StoreError> {
    let row = tx
        .query_row(
            "SELECT kind, start_utc, end_utc, all_day, series_end_utc FROM events
             WHERE account_id = ?1 AND calendar_id = ?2 AND event_id = ?3",
            params![account_id, calendar_id, event_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(|(kind, start, end, all_day, series_end)| {
        span_of(Kind::parse(&kind), start, end, all_day, series_end)
    }))
}
