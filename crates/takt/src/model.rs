//! The UI's view of the cache. Reads run on the background executor; sync
//! events decide when to read again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, TimeDelta, Utc};
use chrono_tz::Tz;
use gpui_kit::{AppContext as _, Context, EventEmitter, Task};
use takt_store::{AccountRow, CalendarRow, Store, StoreError, local_days};
use takt_sync::{SyncEvent, SyncState};

use crate::agenda::{AgendaInput, DaySection, build_agenda};

pub const PAGE_DAYS: i64 = 30;

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub accounts: Vec<AccountRow>,
    pub calendars: Vec<CalendarRow>,
    pub sections: Vec<DaySection>,
}

pub fn load_snapshot(
    db: &Path,
    tz: Tz,
    today: NaiveDate,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Snapshot, StoreError> {
    let store = Store::open_read_only(db)?;
    let accounts = store.accounts()?;
    let calendars = store.calendars(None)?;
    let mut palettes = HashMap::new();
    for a in &accounts {
        palettes.insert(a.account_id.clone(), store.palette(&a.account_id)?);
    }
    let use_24h = match accounts.first() {
        Some(a) => store
            .settings(&a.account_id)?
            .get("format24HourTime")
            .is_none_or(|v| v == "true"),
        None => true,
    };
    let occurrences = store.occurrences(&local_days(from, to, tz), tz)?;
    let tasks = store.tasks_due(from, to)?;
    let sections = build_agenda(&AgendaInput {
        occurrences: &occurrences,
        tasks: &tasks,
        accounts: &accounts,
        calendars: &calendars,
        palettes: &palettes,
        tz,
        today,
        from,
        to,
        use_24h,
    });
    Ok(Snapshot {
        accounts,
        calendars,
        sections,
    })
}

pub enum ModelEvent {
    Reloaded,
}

pub struct CalendarModel {
    db: PathBuf,
    pub tz: Tz,
    today: NaiveDate,
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub snapshot: Snapshot,
    pub statuses: HashMap<String, SyncState>,
    pub last_error: Option<String>,
    loading: Option<Task<()>>,
}

impl EventEmitter<ModelEvent> for CalendarModel {}

impl CalendarModel {
    pub fn new(db: PathBuf, tz: Tz, today: NaiveDate, cx: &mut Context<Self>) -> Self {
        let mut model = Self {
            db,
            tz,
            today,
            from: today,
            to: today + TimeDelta::days(PAGE_DAYS),
            snapshot: Snapshot::default(),
            statuses: HashMap::new(),
            last_error: None,
            loading: None,
        };
        model.reload(cx);
        model
    }

    pub fn db(&self) -> &Path {
        &self.db
    }

    /// Re-reads the loaded range. A newer reload replaces (cancels) an older one.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (db, tz, today, from, to) = (self.db.clone(), self.tz, self.today, self.from, self.to);
        let job = cx.background_spawn(async move { load_snapshot(&db, tz, today, from, to) });
        self.loading = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |model, cx| {
                model.loading = None;
                match result {
                    Ok(snapshot) => model.snapshot = snapshot,
                    Err(error) => {
                        tracing::error!(%error, "cannot read the cache");
                        model.last_error = Some(format!("Cannot read the local cache: {error}"));
                    }
                }
                cx.emit(ModelEvent::Reloaded);
                cx.notify();
            });
        }));
    }

    pub fn load_later(&mut self, cx: &mut Context<Self>) {
        if self.loading.is_none() {
            self.to += TimeDelta::days(PAGE_DAYS);
            self.reload(cx);
        }
    }

    pub fn load_earlier(&mut self, cx: &mut Context<Self>) {
        if self.loading.is_none() {
            self.from -= TimeDelta::days(PAGE_DAYS);
            self.reload(cx);
        }
    }

    pub fn jump_to_today(&mut self, cx: &mut Context<Self>) {
        self.today = Utc::now().with_timezone(&self.tz).date_naive();
        self.from = self.today;
        self.to = self.today + TimeDelta::days(PAGE_DAYS);
        self.reload(cx);
    }

    pub fn handle_sync_event(&mut self, event: SyncEvent, cx: &mut Context<Self>) {
        match event {
            SyncEvent::Changed { changes, .. } => {
                let range = local_days(self.from, self.to, self.tz);
                if changes.affects(range.start, range.end) {
                    self.reload(cx);
                }
            }
            SyncEvent::Status { account_id, state } => {
                self.statuses.insert(account_id, state);
                cx.notify();
            }
            SyncEvent::AuthNeeded { account_id } => {
                self.statuses.insert(account_id, SyncState::NeedsReauth);
                self.reload(cx);
            }
            SyncEvent::AccountsChanged => self.reload(cx),
            SyncEvent::SignInFailed(message) => {
                self.last_error = Some(message);
                cx.notify();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, Utc};
    use serde_json::json;
    use takt_google::model::{CalendarListEntry, EventFields, Item};
    use takt_store::Store;

    use super::*;

    #[test]
    fn snapshot_reads_what_sync_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("takt.sqlite3");
        let mut store = Store::open(&db).unwrap();
        store.upsert_account("acc", "me@example.com").unwrap();
        let cal: Item<CalendarListEntry> =
            Item::from_value(json!({"id": "cal", "selected": true})).unwrap();
        store.apply_calendar_list_page("acc", &[cal], None).unwrap();
        let start = Utc::now() + TimeDelta::hours(1);
        let event: Item<EventFields> = Item::from_value(json!({
            "id": "e", "summary": "Soon",
            "start": {"dateTime": start.to_rfc3339()}, "end": {"dateTime": (start + TimeDelta::hours(1)).to_rfc3339()}
        }))
        .unwrap();
        store
            .apply_events_page("acc", "cal", &[event], None)
            .unwrap();

        let today = Utc::now().date_naive();
        let snap = load_snapshot(
            &db,
            chrono_tz::UTC,
            today,
            today,
            today + TimeDelta::days(PAGE_DAYS),
        )
        .unwrap();
        assert_eq!(snap.accounts.len(), 1);
        assert!(
            snap.sections
                .iter()
                .flat_map(|s| &s.rows)
                .any(|r| r.title == "Soon")
        );
    }
}
