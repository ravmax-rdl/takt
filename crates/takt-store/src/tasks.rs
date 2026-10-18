use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};
use rusqlite::{OptionalExtension, params};
use takt_google::model::{Item, TaskFields, TaskListFields};

use crate::{ChangeSet, Store, StoreError, TaskItem, TimeSpan};

impl Store {
    /// Applies the complete list of task lists for an account.
    pub fn apply_task_lists(
        &mut self,
        account_id: &str,
        lists: &[Item<TaskListFields>],
    ) -> Result<ChangeSet, StoreError> {
        let tx = self.conn.transaction()?;
        let mut changes = ChangeSet::default();
        let keep: HashSet<&str> = lists.iter().map(|l| l.fields.id.as_str()).collect();
        let existing: Vec<String> = {
            let mut stmt = tx.prepare("SELECT list_id FROM task_lists WHERE account_id = ?1")?;
            let rows = stmt.query_map(params![account_id], |row| row.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        for gone in existing.iter().filter(|id| !keep.contains(id.as_str())) {
            tx.execute(
                "DELETE FROM task_lists WHERE account_id = ?1 AND list_id = ?2",
                params![account_id, gone],
            )?;
            changes.touch_task_list(gone, None);
        }
        for l in lists {
            tx.execute(
                "INSERT INTO task_lists (account_id, list_id, raw_json, title, updated) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (account_id, list_id) DO UPDATE SET
                     raw_json = excluded.raw_json, title = excluded.title, updated = excluded.updated",
                params![
                    account_id,
                    l.fields.id,
                    l.raw.to_string(),
                    l.fields.title.clone().unwrap_or_default(),
                    l.fields.updated.map(|u| u.to_rfc3339()),
                ],
            )?;
        }
        tx.commit()?;
        Ok(changes)
    }

    pub fn task_checkpoint(
        &self,
        account_id: &str,
        list_id: &str,
    ) -> Result<Option<DateTime<Utc>>, StoreError> {
        let text: Option<String> = self
            .conn
            .query_row(
                "SELECT updated_min_checkpoint FROM task_lists WHERE account_id = ?1 AND list_id = ?2",
                params![account_id, list_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        text.map(|t| {
            DateTime::parse_from_rfc3339(&t)
                .map(|d| d.with_timezone(&Utc))
                .map_err(|e| StoreError::Corrupt(format!("checkpoint {t}: {e}")))
        })
        .transpose()
    }

    pub fn apply_tasks_page(
        &mut self,
        account_id: &str,
        list_id: &str,
        items: &[Item<TaskFields>],
    ) -> Result<ChangeSet, StoreError> {
        let mut checkpoint = self.task_checkpoint(account_id, list_id)?;
        let tx = self.conn.transaction()?;
        let mut changes = ChangeSet::default();
        for item in items {
            let t = &item.fields;
            if let Some(updated) = t.updated {
                checkpoint = Some(checkpoint.map_or(updated, |c| c.max(updated)));
            }
            let old_due: Option<String> = tx
                .query_row(
                    "SELECT due_date FROM tasks WHERE account_id = ?1 AND list_id = ?2 AND task_id = ?3",
                    params![account_id, list_id, t.id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            if let Some(date) = old_due
                .as_deref()
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            {
                changes.touch_task_list(list_id, Some(day_span(date)));
            }
            if t.deleted {
                tx.execute(
                    "DELETE FROM tasks WHERE account_id = ?1 AND list_id = ?2 AND task_id = ?3",
                    params![account_id, list_id, t.id],
                )?;
                changes.task_lists.insert(list_id.to_owned());
                continue;
            }
            let due = t.due_date();
            if let Some(date) = due {
                changes.touch_task_list(list_id, Some(day_span(date)));
            }
            tx.execute(
                "INSERT INTO tasks (account_id, list_id, task_id, raw_json, title, status, due_date, parent, position,
                                    completed, hidden, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                 ON CONFLICT (account_id, list_id, task_id) DO UPDATE SET
                     raw_json = excluded.raw_json, title = excluded.title, status = excluded.status,
                     due_date = excluded.due_date, parent = excluded.parent, position = excluded.position,
                     completed = excluded.completed, hidden = excluded.hidden, updated = excluded.updated",
                params![
                    account_id,
                    list_id,
                    t.id,
                    item.raw.to_string(),
                    t.title.clone().unwrap_or_default(),
                    t.status,
                    due.map(|d| d.format("%Y-%m-%d").to_string()),
                    t.parent,
                    t.position,
                    t.completed.map(|c| c.to_rfc3339()),
                    t.hidden,
                    t.updated.map(|u| u.to_rfc3339()),
                ],
            )?;
        }
        if let Some(c) = checkpoint {
            tx.execute(
                "UPDATE task_lists SET updated_min_checkpoint = ?3 WHERE account_id = ?1 AND list_id = ?2",
                params![account_id, list_id, c.to_rfc3339()],
            )?;
        }
        tx.commit()?;
        Ok(changes)
    }

    pub fn tasks_due(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<TaskItem>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT t.account_id, t.list_id, l.title, t.task_id, t.title, t.due_date, t.status
             FROM tasks t JOIN task_lists l ON l.account_id = t.account_id AND l.list_id = t.list_id
             WHERE t.hidden = 0 AND t.due_date >= ?1 AND t.due_date < ?2
             ORDER BY t.due_date, t.title COLLATE NOCASE",
        )?;
        let rows = stmt.query_map(
            params![
                from.format("%Y-%m-%d").to_string(),
                to.format("%Y-%m-%d").to_string()
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )?;
        let mut out = Vec::new();
        for row in rows {
            let (account_id, list_id, list_title, task_id, title, due, status) = row?;
            let due_date = NaiveDate::parse_from_str(&due, "%Y-%m-%d")
                .map_err(|e| StoreError::Corrupt(format!("due date {due}: {e}")))?;
            out.push(TaskItem {
                account_id,
                list_id,
                list_title,
                task_id,
                title,
                due_date,
                completed: status.as_deref() == Some("completed"),
            });
        }
        Ok(out)
    }
}

/// A task's date as a span, a day wider each side for display zones.
fn day_span(date: NaiveDate) -> TimeSpan {
    let start = date.and_time(NaiveTime::MIN).and_utc();
    TimeSpan {
        start: start - TimeDelta::days(1),
        end: Some(start + TimeDelta::days(2)),
    }
}
