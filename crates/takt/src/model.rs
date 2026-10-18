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
/// Scrolling loads later days on its own only up to this far past today;
/// the "Load later" button goes further.
pub const AUTO_LOAD_HORIZON_DAYS: i64 = 365;

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// The days this snapshot covers, `from..to`. `None` before the first read.
    pub range: Option<(NaiveDate, NaiveDate)>,
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
        range: Some((from, to)),
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
    /// Whether `last_error` came from a failed read, so a good read clears it.
    error_from_load: bool,
    loading: Option<Task<()>>,
    /// Bumped by every reload; a finished read only lands if it is the latest.
    generation: u64,
    /// Section count before the pending "later" extension, to tell whether it found anything.
    sections_before_later: Option<usize>,
    /// The last extension to later days found no new day: stop auto-loading.
    later_exhausted: bool,
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
            error_from_load: false,
            loading: None,
            generation: 0,
            sections_before_later: None,
            later_exhausted: false,
        };
        model.reload(cx);
        model
    }

    pub fn db(&self) -> &Path {
        &self.db
    }

    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// Re-reads the loaded range. A newer reload replaces (cancels) an older one.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let (db, tz, today, from, to) = (self.db.clone(), self.tz, self.today, self.from, self.to);
        let job = cx.background_spawn(async move { load_snapshot(&db, tz, today, from, to) });
        self.loading = Some(cx.spawn(async move |this, cx| {
            let result = job.await;
            let _ = this.update(cx, |model, cx| model.finish_load(generation, result, cx));
        }));
    }

    fn finish_load(
        &mut self,
        generation: u64,
        result: Result<Snapshot, StoreError>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        self.loading = None;
        let before_later = self.sections_before_later.take();
        match result {
            Ok(snapshot) => {
                if let Some(before) = before_later {
                    self.later_exhausted = snapshot.sections.len() <= before;
                }
                self.snapshot = snapshot;
                if self.error_from_load {
                    self.last_error = None;
                    self.error_from_load = false;
                }
            }
            Err(error) => {
                tracing::error!(%error, "cannot read the cache");
                // Keep the range in step with what is on screen.
                if let Some((from, to)) = self.snapshot.range {
                    self.from = from;
                    self.to = to;
                }
                self.last_error = Some(format!("Cannot read the local cache: {error}"));
                self.error_from_load = true;
            }
        }
        cx.emit(ModelEvent::Reloaded);
        cx.notify();
    }

    /// Whether scrolling near the bottom may load later days by itself.
    pub fn can_auto_load_later(&self) -> bool {
        self.loading.is_none()
            && !self.later_exhausted
            && self.to < self.today + TimeDelta::days(AUTO_LOAD_HORIZON_DAYS)
    }

    /// The list's scroll-triggered path: never stacks on a running read.
    pub fn auto_load_later(&mut self, cx: &mut Context<Self>) {
        if self.can_auto_load_later() {
            self.load_later(cx);
        }
    }

    /// Extends the range by a page of later days. An explicit request always
    /// proceeds; its reload supersedes any read in flight.
    pub fn load_later(&mut self, cx: &mut Context<Self>) {
        self.to += TimeDelta::days(PAGE_DAYS);
        let shown = self.snapshot.sections.len();
        self.sections_before_later.get_or_insert(shown);
        self.reload(cx);
    }

    pub fn load_earlier(&mut self, cx: &mut Context<Self>) {
        self.from -= TimeDelta::days(PAGE_DAYS);
        self.reload(cx);
    }

    pub fn jump_to_today(&mut self, cx: &mut Context<Self>) {
        self.today = Utc::now().with_timezone(&self.tz).date_naive();
        self.from = self.today;
        self.to = self.today + TimeDelta::days(PAGE_DAYS);
        self.sections_before_later = None;
        self.later_exhausted = false;
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
                self.error_from_load = false;
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
        assert_eq!(
            snap.range,
            Some((today, today + TimeDelta::days(PAGE_DAYS)))
        );
        assert_eq!(snap.accounts.len(), 1);
        assert!(
            snap.sections
                .iter()
                .flat_map(|s| &s.rows)
                .any(|r| r.title == "Soon")
        );
    }

    fn open_model(
        cx: &mut gpui_kit::TestAppContext,
    ) -> (tempfile::TempDir, gpui_kit::Entity<CalendarModel>) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("takt.sqlite3");
        Store::open(&db).unwrap();
        let today = Utc::now().date_naive();
        let model = cx.new(|cx| CalendarModel::new(db, chrono_tz::UTC, today, cx));
        cx.run_until_parked();
        (dir, model)
    }

    fn changed(start: chrono::DateTime<Utc>) -> SyncEvent {
        SyncEvent::Changed {
            account_id: "acc".into(),
            changes: takt_store::ChangeSet {
                calendars: ["cal".to_owned()].into(),
                span: Some(takt_store::TimeSpan {
                    start,
                    end: Some(start + TimeDelta::hours(1)),
                }),
                ..Default::default()
            },
        }
    }

    #[gpui_kit::test]
    fn change_outside_the_window_does_not_reload(cx: &mut gpui_kit::TestAppContext) {
        let (_dir, model) = open_model(cx);
        let generation = cx.read(|cx| model.read(cx).generation);
        model.update(cx, |m, cx| {
            m.handle_sync_event(changed(Utc::now() + TimeDelta::days(400)), cx)
        });
        cx.read(|cx| {
            assert_eq!(model.read(cx).generation, generation);
            assert!(!model.read(cx).is_loading());
        });
    }

    #[gpui_kit::test]
    fn change_inside_the_window_reloads(cx: &mut gpui_kit::TestAppContext) {
        let (_dir, model) = open_model(cx);
        let generation = cx.read(|cx| model.read(cx).generation);
        model.update(cx, |m, cx| {
            m.handle_sync_event(changed(Utc::now() + TimeDelta::hours(2)), cx)
        });
        cx.read(|cx| {
            assert_eq!(model.read(cx).generation, generation + 1);
            assert!(model.read(cx).is_loading());
        });
        cx.run_until_parked();
        assert!(cx.read(|cx| !model.read(cx).is_loading()));
    }

    #[gpui_kit::test]
    fn stale_load_cannot_overwrite_a_newer_one(cx: &mut gpui_kit::TestAppContext) {
        let (_dir, model) = open_model(cx);
        let (from, to) = cx.read(|cx| (model.read(cx).from, model.read(cx).to));
        let stale = cx.read(|cx| model.read(cx).generation);
        model.update(cx, |m, cx| {
            m.load_earlier(cx);
            let fake = Snapshot {
                range: Some((from, from)),
                ..Default::default()
            };
            m.finish_load(stale, Ok(fake), cx);
            assert_eq!(m.snapshot.range, Some((from, to)));
            assert!(m.is_loading());
        });
        cx.run_until_parked();
        assert_eq!(
            cx.read(|cx| model.read(cx).snapshot.range),
            Some((from - TimeDelta::days(PAGE_DAYS), to))
        );
    }

    #[gpui_kit::test]
    fn explicit_loads_proceed_while_a_read_runs(cx: &mut gpui_kit::TestAppContext) {
        let (_dir, model) = open_model(cx);
        let to = cx.read(|cx| model.read(cx).to);
        model.update(cx, |m, cx| {
            m.load_later(cx);
            assert!(m.is_loading());
            m.auto_load_later(cx);
            m.load_later(cx);
        });
        cx.run_until_parked();
        assert_eq!(
            cx.read(|cx| model.read(cx).to),
            to + TimeDelta::days(2 * PAGE_DAYS)
        );
    }

    #[gpui_kit::test]
    fn failed_read_restores_the_range_and_a_good_one_clears_the_error(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        let (_dir, model) = open_model(cx);
        let (from, to) = cx.read(|cx| (model.read(cx).from, model.read(cx).to));
        model.update(cx, |m, cx| {
            m.load_later(cx);
            let current = m.generation;
            m.finish_load(current, Err(StoreError::Corrupt("boom".into())), cx);
            assert_eq!((m.from, m.to), (from, to));
            assert!(m.last_error.is_some());
            assert!(!m.is_loading());
            m.reload(cx);
        });
        cx.run_until_parked();
        assert!(cx.read(|cx| model.read(cx).last_error.is_none()));
    }
}
