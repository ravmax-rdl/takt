use std::collections::BTreeMap;

use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use takt_google::model::{CalendarListEntry, Colors, Item, Setting};

use crate::{AccountRow, AccountStatus, CalendarRow, ChangeSet, ColorPalette, Store, StoreError};

impl Store {
    pub fn upsert_account(&mut self, account_id: &str, email: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO accounts (account_id, email, status, added_at) VALUES (?1, ?2, 'ok', ?3)
             ON CONFLICT (account_id) DO UPDATE SET email = excluded.email, status = 'ok'",
            params![account_id, email, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn accounts(&self) -> Result<Vec<AccountRow>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT account_id, email, status FROM accounts ORDER BY added_at, email")?;
        let rows = stmt.query_map([], |row| {
            Ok(AccountRow {
                account_id: row.get(0)?,
                email: row.get(1)?,
                status: AccountStatus::parse(&row.get::<_, String>(2)?),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_account_status(
        &mut self,
        account_id: &str,
        status: AccountStatus,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE accounts SET status = ?2 WHERE account_id = ?1",
            params![account_id, status.as_str()],
        )?;
        Ok(())
    }

    pub fn remove_account(&mut self, account_id: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM accounts WHERE account_id = ?1",
            params![account_id],
        )?;
        Ok(())
    }

    pub fn calendar_list_sync_token(&self, account_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT calendar_list_sync_token FROM accounts WHERE account_id = ?1",
                params![account_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    /// Applies one page of `calendarList.list`. Uses `ON CONFLICT DO UPDATE`,
    /// never `REPLACE`: a replace would delete the row and cascade to its events.
    pub fn apply_calendar_list_page(
        &mut self,
        account_id: &str,
        items: &[Item<CalendarListEntry>],
        next_sync_token: Option<&str>,
    ) -> Result<ChangeSet, StoreError> {
        let tx = self.conn.transaction()?;
        let mut changes = ChangeSet::default();
        for item in items {
            let c = &item.fields;
            changes.touch_calendar(&c.id, None);
            if c.deleted {
                tx.execute(
                    "DELETE FROM calendars WHERE account_id = ?1 AND calendar_id = ?2",
                    params![account_id, c.id],
                )?;
                continue;
            }
            tx.execute(
                "INSERT INTO calendars (account_id, calendar_id, raw_json, summary, time_zone, background_color,
                     foreground_color, access_role, selected, hidden, is_primary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (account_id, calendar_id) DO UPDATE SET
                     raw_json = excluded.raw_json, summary = excluded.summary, time_zone = excluded.time_zone,
                     background_color = excluded.background_color, foreground_color = excluded.foreground_color,
                     access_role = excluded.access_role, selected = excluded.selected, hidden = excluded.hidden,
                     is_primary = excluded.is_primary",
                params![
                    account_id,
                    c.id,
                    item.raw.to_string(),
                    c.display_name(),
                    c.time_zone,
                    c.background_color,
                    c.foreground_color,
                    c.access_role,
                    c.selected,
                    c.hidden,
                    c.primary,
                ],
            )?;
        }
        if let Some(token) = next_sync_token {
            tx.execute(
                "UPDATE accounts SET calendar_list_sync_token = ?2 WHERE account_id = ?1",
                params![account_id, token],
            )?;
        }
        tx.commit()?;
        Ok(changes)
    }

    /// Calendars for one account, or all accounts. Primary first, then by name.
    pub fn calendars(&self, account_id: Option<&str>) -> Result<Vec<CalendarRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT c.account_id, c.calendar_id, c.summary, c.time_zone, c.background_color, c.foreground_color,
                    c.access_role, c.selected, c.hidden, c.is_primary
             FROM calendars c JOIN accounts a ON a.account_id = c.account_id
             WHERE ?1 IS NULL OR c.account_id = ?1
             ORDER BY a.added_at, a.email, c.is_primary DESC, c.summary COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok(CalendarRow {
                account_id: row.get(0)?,
                calendar_id: row.get(1)?,
                summary: row.get(2)?,
                time_zone: row.get(3)?,
                background_color: row.get(4)?,
                foreground_color: row.get(5)?,
                access_role: row.get(6)?,
                selected: row.get(7)?,
                hidden: row.get(8)?,
                primary: row.get(9)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Drops a calendar and its events, and the account's calendar-list token:
    /// the token no longer describes what is stored, so the next round fetches
    /// the full list (and brings the calendar back if access returns).
    pub fn remove_calendar(
        &mut self,
        account_id: &str,
        calendar_id: &str,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM calendars WHERE account_id = ?1 AND calendar_id = ?2",
            params![account_id, calendar_id],
        )?;
        tx.execute(
            "UPDATE accounts SET calendar_list_sync_token = NULL WHERE account_id = ?1",
            params![account_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// After a 410 on the calendar list: drop the account's calendars (and
    /// their events) and its token, so the next round is a full sync.
    pub fn reset_account_sync(&mut self, account_id: &str) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM calendars WHERE account_id = ?1",
            params![account_id],
        )?;
        tx.execute(
            "UPDATE accounts SET calendar_list_sync_token = NULL WHERE account_id = ?1",
            params![account_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Replaces the account's settings with the full `settings.list` result.
    pub fn put_settings(
        &mut self,
        account_id: &str,
        settings: &[Item<Setting>],
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM account_settings WHERE account_id = ?1",
            params![account_id],
        )?;
        for s in settings {
            tx.execute(
                "INSERT INTO account_settings (account_id, key, raw_json, value) VALUES (?1, ?2, ?3, ?4)",
                params![account_id, s.fields.id, s.raw.to_string(), s.fields.value],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn settings(&self, account_id: &str) -> Result<BTreeMap<String, String>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM account_settings WHERE account_id = ?1")?;
        let rows = stmt.query_map(params![account_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Replaces the account's palette with a `colors.get` response. Each row
    /// keeps that colour's own object from the response.
    pub fn put_colors(
        &mut self,
        account_id: &str,
        colors: &Item<Colors>,
    ) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM colors WHERE account_id = ?1",
            params![account_id],
        )?;
        for (kind, map) in [
            ("calendar", &colors.fields.calendar),
            ("event", &colors.fields.event),
        ] {
            for (id, def) in map {
                let raw = colors
                    .raw
                    .get(kind)
                    .and_then(|entries| entries.get(id))
                    .ok_or_else(|| {
                        StoreError::Corrupt(format!("colour {kind}/{id} missing from its JSON"))
                    })?;
                tx.execute(
                    "INSERT INTO colors (account_id, kind, color_id, raw_json, background, foreground)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![account_id, kind, id, raw.to_string(), def.background, def.foreground],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn palette(&self, account_id: &str) -> Result<ColorPalette, StoreError> {
        let mut palette = ColorPalette::default();
        let mut stmt = self.conn.prepare(
            "SELECT kind, color_id, background, foreground FROM colors WHERE account_id = ?1",
        )?;
        let rows = stmt.query_map(params![account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get(2)?,
                row.get(3)?,
            ))
        })?;
        for row in rows {
            let (kind, id, background, foreground) = row?;
            let map = if kind == "calendar" {
                &mut palette.calendar
            } else {
                &mut palette.event
            };
            map.insert(id, (background, foreground));
        }
        Ok(palette)
    }
}
