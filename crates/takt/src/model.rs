//! The UI's view of the cache. Reads run on the background executor; sync
//! events decide when to read again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, TimeDelta, Utc, Weekday};
use chrono_tz::Tz;
use gpui_kit::{AppContext as _, Context, EventEmitter, Task};
use takt_store::{AccountRow, CalendarRow, Store, StoreError, local_days};
use takt_sync::{SyncEvent, SyncState, WriteError, WriteOutcome};

use crate::agenda::{AgendaInput, DaySection, Styling, build_agenda};
use crate::grid::{GridInput, MonthGrid, TimeGrid, build_month_grid, build_time_grid};
pub use crate::view::PAGE_DAYS;
use crate::view::{ViewKind, parse_week_start, step, visible_days};

/// Scrolling loads later days on its own only up to this far past today;
/// the "Load later" button goes further.
pub const AUTO_LOAD_HORIZON_DAYS: i64 = 365;

/// How the model starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelOptions {
    /// Every time is shown in this zone.
    pub tz: Tz,
    /// A second zone for the time grid's hour labels.
    pub secondary_tz: Option<Tz>,
    /// Fixed week start; `None` follows the Google account's setting.
    pub week_start: Option<Weekday>,
    pub view: ViewKind,
}

impl ModelOptions {
    pub fn new(tz: Tz, view: ViewKind) -> Self {
        Self {
            tz,
            secondary_tz: None,
            week_start: None,
            view,
        }
    }
}

/// One read of the cache: the days `from..to`, built for `view`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Query {
    pub tz: Tz,
    pub secondary_tz: Option<Tz>,
    pub today: NaiveDate,
    pub view: ViewKind,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// The days this snapshot covers, `from..to`. `None` before the first read.
    pub range: Option<(NaiveDate, NaiveDate)>,
    /// The view this snapshot was built for; only that view's part is filled.
    pub view: ViewKind,
    pub accounts: Vec<AccountRow>,
    pub calendars: Vec<CalendarRow>,
    /// Agenda.
    pub sections: Vec<DaySection>,
    /// Day and week.
    pub grid: TimeGrid,
    pub month: MonthGrid,
    /// The first account's "week starts on" setting, when it has one.
    pub week_start: Option<Weekday>,
    /// Times are written as 13:00, not 1:00 PM.
    pub use_24h: bool,
}

pub fn load_snapshot(db: &Path, query: &Query) -> Result<Snapshot, StoreError> {
    let Query {
        tz,
        today,
        from,
        to,
        ..
    } = *query;
    let store = Store::open_read_only(db)?;
    let accounts = store.accounts()?;
    let calendars = store.calendars(None)?;
    let mut palettes = HashMap::new();
    for a in &accounts {
        palettes.insert(a.account_id.clone(), store.palette(&a.account_id)?);
    }
    let settings = match accounts.first() {
        Some(a) => store.settings(&a.account_id)?,
        None => Default::default(),
    };
    let use_24h = settings.get("format24HourTime").is_none_or(|v| v == "true");
    let week_start = settings.get("weekStart").and_then(|v| parse_week_start(v));
    let occurrences = store.occurrences(&local_days(from, to, tz), tz)?;
    let mut snapshot = Snapshot {
        range: Some((from, to)),
        view: query.view,
        week_start,
        use_24h,
        ..Default::default()
    };
    if query.view == ViewKind::Agenda {
        let tasks = store.tasks_due(from, to)?;
        snapshot.sections = build_agenda(&AgendaInput {
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
    } else {
        let styling = Styling::new(&accounts, &calendars, &palettes);
        let input = GridInput {
            occurrences: &occurrences,
            styling: &styling,
            tz,
            today,
            from,
            to,
            use_24h,
        };
        if query.view == ViewKind::Month {
            snapshot.month = build_month_grid(&input);
        } else {
            snapshot.grid = build_time_grid(&input, query.secondary_tz);
        }
    }
    snapshot.accounts = accounts;
    snapshot.calendars = calendars;
    Ok(snapshot)
}

pub enum ModelEvent {
    Reloaded,
    /// Sync answered a write the view sent. On success the change it made
    /// was announced first, so a read for it is already running.
    WriteDone {
        id: u64,
        result: Result<WriteOutcome, WriteError>,
    },
}

pub struct CalendarModel {
    db: PathBuf,
    pub tz: Tz,
    secondary_tz: Option<Tz>,
    /// From config.toml; when set, Google's setting is ignored.
    week_start_override: Option<Weekday>,
    week_start: Weekday,
    today: NaiveDate,
    view: ViewKind,
    /// The day the view is built around, and the selected day in a grid.
    anchor: NaiveDate,
    /// The days being shown or loaded, `from..to`.
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
    pub fn new(
        db: PathBuf,
        options: ModelOptions,
        today: NaiveDate,
        cx: &mut Context<Self>,
    ) -> Self {
        let week_start = options.week_start.unwrap_or(Weekday::Mon);
        let (from, to) = visible_days(options.view, today, week_start);
        let mut model = Self {
            db,
            tz: options.tz,
            secondary_tz: options.secondary_tz,
            week_start_override: options.week_start,
            week_start,
            today,
            view: options.view,
            anchor: today,
            from,
            to,
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

    pub fn view(&self) -> ViewKind {
        self.view
    }

    pub fn anchor(&self) -> NaiveDate {
        self.anchor
    }

    pub fn week_start(&self) -> Weekday {
        self.week_start
    }

    /// Re-reads the loaded range. A newer reload replaces (cancels) an older one.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let db = self.db.clone();
        let query = Query {
            tz: self.tz,
            secondary_tz: self.secondary_tz,
            today: self.today,
            view: self.view,
            from: self.from,
            to: self.to,
        };
        let job = cx.background_spawn(async move { load_snapshot(&db, &query) });
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
                let google_week_start = snapshot.week_start;
                self.snapshot = snapshot;
                if self.error_from_load {
                    self.last_error = None;
                    self.error_from_load = false;
                }
                // The account's week start is only known after a read; a week
                // or month laid out on the wrong one is read again.
                if self.week_start_override.is_none()
                    && let Some(week_start) = google_week_start
                    && week_start != self.week_start
                {
                    self.week_start = week_start;
                    if matches!(self.view, ViewKind::Week | ViewKind::Month) {
                        self.set_range();
                        self.reload(cx);
                    }
                }
            }
            Err(error) => {
                tracing::error!(%error, "cannot read the cache");
                // Keep the view and range in step with what is on screen.
                if let Some((from, to)) = self.snapshot.range {
                    self.view = self.snapshot.view;
                    self.from = from;
                    self.to = to;
                    if self.view != ViewKind::Agenda {
                        self.anchor = self.anchor.clamp(from, to - TimeDelta::days(1));
                    }
                }
                self.last_error = Some(format!("Cannot read the local cache: {error}"));
                self.error_from_load = true;
            }
        }
        cx.emit(ModelEvent::Reloaded);
        cx.notify();
    }

    /// Points the range at what the view shows around the anchor.
    fn set_range(&mut self) {
        (self.from, self.to) = visible_days(self.view, self.anchor, self.week_start);
        self.sections_before_later = None;
        self.later_exhausted = false;
    }

    pub fn set_view(&mut self, view: ViewKind, cx: &mut Context<Self>) {
        if view == self.view {
            return;
        }
        self.view = view;
        self.set_range();
        self.reload(cx);
    }

    /// Moves the anchor to `date`. A grid reads again only when `date` is
    /// outside the days it already shows; the agenda restarts at `date`.
    pub fn go_to(&mut self, date: NaiveDate, cx: &mut Context<Self>) {
        let shown = (self.from, self.to);
        self.anchor = date;
        self.set_range();
        if self.view == ViewKind::Agenda || (self.from, self.to) != shown {
            self.reload(cx);
        } else {
            cx.notify();
        }
    }

    /// Moves by `count` days, weeks or months, whichever the view shows.
    pub fn step(&mut self, count: i64, cx: &mut Context<Self>) {
        self.go_to(step(self.view, self.anchor, count), cx);
    }

    /// Whether scrolling near the bottom may load later days by itself.
    pub fn can_auto_load_later(&self) -> bool {
        self.view == ViewKind::Agenda
            && self.loading.is_none()
            && !self.later_exhausted
            && self.to < self.today + TimeDelta::days(AUTO_LOAD_HORIZON_DAYS)
    }

    /// The list's scroll-triggered path: never stacks on a running read.
    pub fn auto_load_later(&mut self, cx: &mut Context<Self>) {
        if self.can_auto_load_later() {
            self.load_later(cx);
        }
    }

    /// Extends the agenda by a page of later days. An explicit request always
    /// proceeds; its reload supersedes any read in flight.
    pub fn load_later(&mut self, cx: &mut Context<Self>) {
        self.to += TimeDelta::days(PAGE_DAYS);
        let shown = self.snapshot.sections.len();
        self.sections_before_later.get_or_insert(shown);
        self.reload(cx);
    }

    /// Extends the agenda by a page of earlier days.
    pub fn load_earlier(&mut self, cx: &mut Context<Self>) {
        self.from -= TimeDelta::days(PAGE_DAYS);
        self.reload(cx);
    }

    pub fn today(&self) -> NaiveDate {
        self.today
    }

    /// Re-reads the clock so "Today" moves on after midnight. Reloads only
    /// when the date changed.
    pub fn refresh_today(&mut self, cx: &mut Context<Self>) {
        let today = Utc::now().with_timezone(&self.tz).date_naive();
        self.set_today(today, cx);
    }

    fn set_today(&mut self, today: NaiveDate, cx: &mut Context<Self>) {
        if today != self.today {
            self.today = today;
            self.reload(cx);
        }
    }

    pub fn jump_to_today(&mut self, cx: &mut Context<Self>) {
        self.today = Utc::now().with_timezone(&self.tz).date_naive();
        self.anchor = self.today;
        self.set_range();
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
            SyncEvent::WriteDone { id, result } => cx.emit(ModelEvent::WriteDone { id, result }),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeDelta, Utc};
    use serde_json::json;
    use takt_google::model::{CalendarListEntry, EventFields, Item, Setting};
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
        let query = Query {
            tz: chrono_tz::UTC,
            secondary_tz: None,
            today,
            view: ViewKind::Agenda,
            from: today,
            to: today + TimeDelta::days(PAGE_DAYS),
        };
        let snap = load_snapshot(&db, &query).unwrap();
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

        // The same days as a time grid and as a month.
        let week = Query {
            view: ViewKind::Week,
            ..query
        };
        let week = load_snapshot(&db, &week).unwrap();
        assert!(week.sections.is_empty());
        assert_eq!(week.grid.days.len() as i64, PAGE_DAYS);
        assert_eq!(week.grid.order.len(), 1);
        let month = Query {
            view: ViewKind::Month,
            ..query
        };
        let month = load_snapshot(&db, &month).unwrap();
        assert!(month.grid.days.is_empty());
        assert_eq!(month.month.order.len(), 1);
    }

    fn open_model(
        cx: &mut gpui_kit::TestAppContext,
    ) -> (tempfile::TempDir, gpui_kit::Entity<CalendarModel>) {
        open_model_in(ModelOptions::new(chrono_tz::UTC, ViewKind::Agenda), cx)
    }

    fn open_model_in(
        options: ModelOptions,
        cx: &mut gpui_kit::TestAppContext,
    ) -> (tempfile::TempDir, gpui_kit::Entity<CalendarModel>) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("takt.sqlite3");
        Store::open(&db).unwrap();
        let today = Utc::now().date_naive();
        let model = cx.new(|cx| CalendarModel::new(db, options, today, cx));
        cx.run_until_parked();
        (dir, model)
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
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
    fn a_new_date_reloads_and_the_same_date_does_not(cx: &mut gpui_kit::TestAppContext) {
        let (_dir, model) = open_model(cx);
        let (today, generation) = cx.read(|cx| (model.read(cx).today(), model.read(cx).generation));
        model.update(cx, |m, cx| m.set_today(today, cx));
        assert_eq!(cx.read(|cx| model.read(cx).generation), generation);

        let tomorrow = today + TimeDelta::days(1);
        model.update(cx, |m, cx| m.set_today(tomorrow, cx));
        cx.run_until_parked();
        cx.read(|cx| {
            let m = model.read(cx);
            assert_eq!(m.today(), tomorrow);
            assert_eq!(m.generation, generation + 1);
            assert!(!m.is_loading());
        });
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

    #[gpui_kit::test]
    fn a_grid_reads_again_only_when_the_anchor_leaves_its_days(cx: &mut gpui_kit::TestAppContext) {
        let (_dir, model) = open_model_in(ModelOptions::new(chrono_tz::UTC, ViewKind::Week), cx);
        // Wednesday 21 October 2026, in the Monday week of the 19th.
        model.update(cx, |m, cx| m.go_to(d(2026, 10, 21), cx));
        cx.run_until_parked();
        let generation = cx.read(|cx| model.read(cx).generation);
        cx.read(|cx| {
            let m = model.read(cx);
            assert_eq!((m.from, m.to), (d(2026, 10, 19), d(2026, 10, 26)));
            assert_eq!(m.snapshot.range, Some((m.from, m.to)));
            assert_eq!(m.snapshot.grid.days.len(), 7);
        });

        model.update(cx, |m, cx| m.go_to(d(2026, 10, 25), cx));
        cx.read(|cx| {
            assert_eq!(model.read(cx).anchor(), d(2026, 10, 25));
            assert_eq!(model.read(cx).generation, generation, "same week: no read");
        });

        model.update(cx, |m, cx| m.step(1, cx));
        cx.run_until_parked();
        cx.read(|cx| {
            let m = model.read(cx);
            assert_eq!(m.anchor(), d(2026, 11, 1));
            assert_eq!((m.from, m.to), (d(2026, 10, 26), d(2026, 11, 2)));
            assert_eq!(m.generation, generation + 1);
        });
    }

    #[gpui_kit::test]
    fn switching_views_keeps_the_anchor(cx: &mut gpui_kit::TestAppContext) {
        let (_dir, model) = open_model_in(ModelOptions::new(chrono_tz::UTC, ViewKind::Week), cx);
        let anchor = d(2026, 10, 21);
        model.update(cx, |m, cx| m.go_to(anchor, cx));
        let ranges = [
            (ViewKind::Day, (anchor, d(2026, 10, 22))),
            (ViewKind::Month, (d(2026, 9, 28), d(2026, 11, 2))),
            (ViewKind::Agenda, (anchor, d(2026, 11, 20))),
            (ViewKind::Week, (d(2026, 10, 19), d(2026, 10, 26))),
        ];
        for (view, range) in ranges {
            model.update(cx, |m, cx| m.set_view(view, cx));
            cx.run_until_parked();
            cx.read(|cx| {
                let m = model.read(cx);
                assert_eq!(m.view(), view);
                assert_eq!(m.anchor(), anchor);
                assert_eq!((m.from, m.to), range);
                assert_eq!(m.snapshot.view, view);
                assert_eq!(m.snapshot.range, Some(range));
            });
        }
        // Month steps keep the day of the month.
        model.update(cx, |m, cx| {
            m.set_view(ViewKind::Month, cx);
            m.step(-1, cx);
        });
        assert_eq!(cx.read(|cx| model.read(cx).anchor()), d(2026, 9, 21));
    }

    fn set_week_start(db: &Path, value: &str) {
        let mut store = Store::open(db).unwrap();
        store.upsert_account("acc", "me@example.com").unwrap();
        let setting: Item<Setting> =
            Item::from_value(json!({"id": "weekStart", "value": value})).unwrap();
        store.put_settings("acc", &[setting]).unwrap();
    }

    #[gpui_kit::test]
    fn the_week_follows_the_accounts_week_start(cx: &mut gpui_kit::TestAppContext) {
        let (_dir, model) = open_model_in(ModelOptions::new(chrono_tz::UTC, ViewKind::Week), cx);
        model.update(cx, |m, cx| m.go_to(d(2026, 10, 21), cx));
        cx.run_until_parked();
        let db = cx.read(|cx| model.read(cx).db().to_owned());
        set_week_start(&db, "0");

        model.update(cx, |m, cx| m.reload(cx));
        cx.run_until_parked();
        cx.read(|cx| {
            let m = model.read(cx);
            assert_eq!(m.week_start(), Weekday::Sun);
            assert_eq!((m.from, m.to), (d(2026, 10, 18), d(2026, 10, 25)));
            assert_eq!(m.snapshot.range, Some((m.from, m.to)));
            assert!(!m.is_loading());
        });
    }

    #[gpui_kit::test]
    fn a_configured_week_start_beats_the_accounts(cx: &mut gpui_kit::TestAppContext) {
        let options = ModelOptions {
            week_start: Some(Weekday::Sat),
            ..ModelOptions::new(chrono_tz::UTC, ViewKind::Week)
        };
        let (_dir, model) = open_model_in(options, cx);
        let db = cx.read(|cx| model.read(cx).db().to_owned());
        set_week_start(&db, "0");
        model.update(cx, |m, cx| m.go_to(d(2026, 10, 21), cx));
        cx.run_until_parked();
        cx.read(|cx| {
            let m = model.read(cx);
            assert_eq!(m.week_start(), Weekday::Sat);
            assert_eq!((m.from, m.to), (d(2026, 10, 17), d(2026, 10, 24)));
        });
    }

    #[gpui_kit::test]
    fn a_failed_read_after_a_view_switch_keeps_the_old_view(cx: &mut gpui_kit::TestAppContext) {
        let (_dir, model) = open_model_in(ModelOptions::new(chrono_tz::UTC, ViewKind::Week), cx);
        let (from, to) = cx.read(|cx| (model.read(cx).from, model.read(cx).to));
        model.update(cx, |m, cx| {
            m.set_view(ViewKind::Month, cx);
            let current = m.generation;
            m.finish_load(current, Err(StoreError::Corrupt("boom".into())), cx);
            assert_eq!(m.view(), ViewKind::Week);
            assert_eq!((m.from, m.to), (from, to));
            assert!(m.last_error.is_some());
        });
    }
}
