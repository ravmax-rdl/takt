use std::path::PathBuf;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use chrono::{Datelike as _, NaiveDate, TimeDelta, Timelike as _, Utc};
use gpui_kit::component::list::{List, ListEvent, ListState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, TITLE_BAR_HEIGHT, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use takt_sync::{Command, SyncEvent};

use crate::agenda::RowKey;
use crate::config::ViewSettings;
use crate::grid::Chip;
use crate::keys::{
    CONTEXT, CloseDetail, GRID_CONTEXT, GridDown, GridUp, JumpToToday, NextDay, NextEvent,
    NextPeriod, OpenSelected, PreviousDay, PreviousEvent, PreviousPeriod, PreviousRow, ShowAgenda,
    ShowDay, ShowMonth, ShowWeek, SyncNow,
};
use crate::model::{CalendarModel, ModelEvent, ModelOptions, Snapshot};
use crate::ui::agenda_list::AgendaDelegate;
use crate::ui::detail_panel::{self, DetailContent};
use crate::ui::grid_common::GridState;
use crate::ui::mini_calendar::{self, MiniCalendar, shift_month};
use crate::ui::toolbar::{self, TOOLBAR_HEIGHT};
use crate::ui::{month_grid, sidebar, status_bar, time_grid};
use crate::view::{self, ViewKind};

/// How often the status bar's "synced 12s ago" texts are redrawn.
const STATUS_TICK: Duration = Duration::from_secs(15);
/// Roughly what the status bar takes of the window's height.
const STATUS_BAR_HEIGHT: f32 = 26.;

pub struct MainView {
    model: Entity<CalendarModel>,
    list: Entity<ListState<AgendaDelegate>>,
    commands: Sender<Command>,
    /// Holds the keyboard focus while a grid is shown.
    grid_focus: FocusHandle,
    grid_scroll: ScrollHandle,
    /// Scroll the time grid to the current hour on its next draw.
    scroll_pending: bool,
    /// The event selected in a grid.
    selected: Option<RowKey>,
    /// The first day of the month the mini calendar shows.
    mini_month: NaiveDate,
    detail: Option<DetailContent>,
    /// The row or event `detail` was opened from.
    detail_key: Option<RowKey>,
    show_raw: bool,
    /// The read behind the detail being opened; a newer open replaces (cancels) it.
    detail_load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

fn month_start(date: NaiveDate) -> NaiveDate {
    date.with_day(1).unwrap_or(date)
}

/// The chip drawn for `key` in the snapshot's grid, wherever it is.
fn find_chip<'a>(snapshot: &'a Snapshot, key: &RowKey) -> Option<&'a Chip> {
    let grid = &snapshot.grid;
    grid.days
        .iter()
        .flat_map(|day| day.blocks.iter().map(|block| &block.chip))
        .chain(grid.bars.iter().map(|bar| &bar.chip))
        .chain(
            snapshot
                .month
                .weeks
                .iter()
                .flat_map(|week| week.bars.iter().map(|bar| &bar.chip)),
        )
        .find(|chip| chip.key == *key)
}

/// The shown grid's events in reading order, each with its first shown day.
fn reading_order(snapshot: &Snapshot) -> &[(NaiveDate, RowKey)] {
    match snapshot.view {
        ViewKind::Month => &snapshot.month.order,
        ViewKind::Day | ViewKind::Week => &snapshot.grid.order,
        ViewKind::Agenda => &[],
    }
}

/// The event `delta` steps from `selected` in `order`, wrapping around. With
/// nothing selected, the first event on or after `anchor` going forward, or
/// the last on or before it going back.
fn neighbour<'a>(
    order: &'a [(NaiveDate, RowKey)],
    selected: Option<&RowKey>,
    anchor: NaiveDate,
    delta: isize,
) -> Option<&'a (NaiveDate, RowKey)> {
    if order.is_empty() {
        return None;
    }
    let len = order.len() as isize;
    let current = selected.and_then(|key| order.iter().position(|(_, k)| k == key));
    let ix = match current {
        Some(ix) => (ix as isize + delta).rem_euclid(len),
        None if delta >= 0 => order
            .iter()
            .position(|(date, _)| *date >= anchor)
            .unwrap_or(0) as isize,
        None => order
            .iter()
            .rposition(|(date, _)| *date <= anchor)
            .map_or(len - 1, |ix| ix as isize),
    };
    order.get(ix as usize)
}

impl MainView {
    pub fn new(
        db: PathBuf,
        settings: ViewSettings,
        commands: Sender<Command>,
        events: Receiver<SyncEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tz = settings.time_zone.unwrap_or_else(crate::display_zone);
        let today = Utc::now().with_timezone(&tz).date_naive();
        let options = ModelOptions {
            tz,
            secondary_tz: settings.secondary_time_zone,
            week_start: settings.week_start,
            view: settings.default_view,
        };
        let model = cx.new(|cx| CalendarModel::new(db, options, today, cx));
        let delegate = AgendaDelegate {
            model: model.clone(),
            sections: Vec::new(),
            selected: None,
        };
        let list = cx.new(|cx| ListState::new(delegate, window, cx).selectable(true));

        let subscriptions = vec![
            cx.subscribe_in(
                &model,
                window,
                |this, _, event: &ModelEvent, window, cx| match event {
                    ModelEvent::Reloaded => this.on_reloaded(window, cx),
                    ModelEvent::WriteDone { .. } => {}
                },
            ),
            cx.subscribe_in(&list, window, |this, _, event: &ListEvent, window, cx| {
                this.on_list_event(event, window, cx)
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                let active = window.is_window_active();
                let _ = this.commands.try_send(Command::WindowFocus(active));
                if active {
                    this.model.update(cx, |m, cx| m.refresh_today(cx));
                }
            }),
        ];

        // Weak, so the bridge ends once the model is gone.
        let bridge_model = model.downgrade();
        let bridge = cx.spawn(async move |_, cx| {
            while let Ok(event) = events.recv().await {
                if bridge_model
                    .update(cx, |m, cx| m.handle_sync_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        });

        // Redraws the relative sync times and the current-time line, and moves
        // "Today" on past midnight while the window stays focused.
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(STATUS_TICK).await;
                let tick = this.update(cx, |view, cx| {
                    view.model.update(cx, |m, cx| m.refresh_today(cx));
                    cx.notify();
                });
                if tick.is_err() {
                    break;
                }
            }
        });

        let view = Self {
            model,
            list,
            commands,
            grid_focus: cx.focus_handle(),
            grid_scroll: ScrollHandle::new(),
            scroll_pending: true,
            selected: None,
            mini_month: month_start(today),
            detail: None,
            detail_key: None,
            show_raw: false,
            detail_load: None,
            _subscriptions: subscriptions,
            _tasks: vec![bridge, ticker],
        };
        view.focus_view(window, cx);
        view
    }

    pub fn model(&self) -> &Entity<CalendarModel> {
        &self.model
    }

    /// The event selected in a grid, if any.
    pub fn selected_event(&self) -> Option<&RowKey> {
        self.selected.as_ref()
    }

    /// Gives the keyboard to whichever of the agenda list and the grid is shown.
    fn focus_view(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.read(cx).view() == ViewKind::Agenda {
            self.list.update(cx, |list, cx| list.focus(window, cx));
        } else {
            self.grid_focus.focus(window, cx);
        }
    }

    fn on_reloaded(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_list(window, cx);
        let snapshot = &self.model.read(cx).snapshot;
        // An event that left the grid is no longer selected.
        if snapshot.view != ViewKind::Agenda
            && let Some(key) = &self.selected
            && !reading_order(snapshot).iter().any(|(_, k)| k == key)
        {
            self.selected = None;
        }
        cx.notify();
    }

    /// Copies new sections into the list and keeps the same row, on the same
    /// day, selected.
    fn refresh_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let snapshot = &self.model.read(cx).snapshot;
        let sections = snapshot.sections.clone();
        let account_gone = |key: &RowKey| match key {
            RowKey::Event { account_id, .. } | RowKey::Task { account_id, .. } => !snapshot
                .accounts
                .iter()
                .any(|a| &a.account_id == account_id),
        };
        if self.detail_key.as_ref().is_some_and(account_gone) {
            self.detail_load = None;
            self.detail = None;
            self.detail_key = None;
        }
        self.list.update(cx, |list, cx| {
            let selected = list.selected_index().and_then(|ix| {
                let delegate = list.delegate();
                Some((delegate.date_of(ix)?, delegate.row(ix)?.key.clone()))
            });
            list.delegate_mut().sections = sections;
            let ix = selected.and_then(|(date, key)| list.delegate().index_of(date, &key));
            list.set_selected_index(ix, window, cx);
            cx.notify();
        });
    }

    fn on_list_event(&mut self, event: &ListEvent, _: &mut Window, cx: &mut Context<Self>) {
        match event {
            ListEvent::Confirm(ix) => self.open_detail(*ix, cx),
            ListEvent::Cancel | ListEvent::Select(_) => {}
        }
    }

    /// Opens the details of an agenda row.
    fn open_detail(&mut self, ix: IndexPath, cx: &mut Context<Self>) {
        let Some((row, header)) = self
            .list
            .read(cx)
            .delegate()
            .sections
            .get(ix.section)
            .and_then(|s| s.rows.get(ix.row).map(|r| (r.clone(), s.header.clone())))
        else {
            return;
        };
        let when = format!("{header}, {}", row.time_label);
        match row.key {
            RowKey::Task { .. } => {
                self.detail_load = None;
                self.detail_key = Some(row.key.clone());
                self.detail = Some(DetailContent::Task {
                    title: row.title.clone(),
                    when,
                    list: row.location.clone().unwrap_or_default(),
                    completed: row.completed,
                });
                self.show_raw = false;
                cx.notify();
            }
            RowKey::Event { .. } => self.open_event_detail(row.key, when, cx),
        }
    }

    /// Reads an event's JSON in the background and shows it in the panel.
    fn open_event_detail(&mut self, key: RowKey, when: String, cx: &mut Context<Self>) {
        let RowKey::Event {
            account_id,
            calendar_id,
            event_id,
            ..
        } = key.clone()
        else {
            return;
        };
        let model = self.model.read(cx);
        let db = model.db().to_path_buf();
        let (tz, use_24h) = (model.tz, model.snapshot.use_24h);
        let calendar = model
            .snapshot
            .calendars
            .iter()
            .find(|c| c.account_id == account_id && c.calendar_id == calendar_id)
            .map(|c| c.summary.clone())
            .unwrap_or_default();
        let account = model
            .snapshot
            .accounts
            .iter()
            .find(|a| a.account_id == account_id)
            .map(|a| a.email.clone())
            .unwrap_or_default();
        let read = cx.background_spawn(async move {
            takt_store::Store::open_read_only(&db)?.event_raw(&account_id, &calendar_id, &event_id)
        });
        self.detail_load = Some(cx.spawn(async move |this, cx| {
            let raw = read.await;
            let _ = this.update(cx, |view, cx| {
                view.detail_load = None;
                match raw {
                    Ok(Some(raw)) => {
                        let mut detail = crate::detail::event_detail(&raw, when, calendar, account);
                        detail.zone_note = crate::detail::zone_note(&raw, tz, use_24h);
                        view.detail = Some(DetailContent::Event(Box::new(detail)));
                        view.detail_key = Some(key);
                        view.show_raw = false;
                    }
                    Ok(None) => {
                        view.detail = None;
                        view.detail_key = None;
                    }
                    Err(error) => {
                        tracing::error!(%error, "cannot read event details");
                        view.detail = None;
                        view.detail_key = None;
                    }
                }
                cx.notify();
            });
        }));
    }

    /// A click on an event in a grid: selects it and opens its details.
    pub(crate) fn select_event(
        &mut self,
        key: RowKey,
        when: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = Some(key.clone());
        self.open_event_detail(key, when, cx);
        self.focus_view(window, cx);
        cx.notify();
    }

    /// The mini calendar follows the selected day.
    fn after_move(&mut self, cx: &mut Context<Self>) {
        self.mini_month = month_start(self.model.read(cx).anchor());
        cx.notify();
    }

    pub(crate) fn show_view(
        &mut self,
        view: ViewKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let had_time_grid = self.model.read(cx).view().has_time_grid();
        self.model.update(cx, |m, cx| m.set_view(view, cx));
        if view.has_time_grid() && !had_time_grid {
            self.scroll_pending = true;
        }
        self.focus_view(window, cx);
        cx.notify();
    }

    /// Selects `date`, moving the view when it does not show it.
    pub(crate) fn go_to(&mut self, date: NaiveDate, window: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |m, cx| m.go_to(date, cx));
        self.focus_view(window, cx);
        self.after_move(cx);
    }

    /// Shows `date` in the day view.
    pub(crate) fn open_day(
        &mut self,
        date: NaiveDate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.model.update(cx, |m, cx| m.go_to(date, cx));
        self.show_view(ViewKind::Day, window, cx);
        self.after_move(cx);
    }

    /// Moves by `count` of the view's periods.
    pub(crate) fn step(&mut self, count: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |m, cx| m.step(count, cx));
        self.focus_view(window, cx);
        self.after_move(cx);
    }

    pub(crate) fn go_today(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |m, cx| m.jump_to_today(cx));
        self.scroll_pending = true;
        self.focus_view(window, cx);
        self.after_move(cx);
    }

    pub(crate) fn shift_mini_month(&mut self, count: i32, cx: &mut Context<Self>) {
        self.mini_month = shift_month(self.mini_month, count);
        cx.notify();
    }

    /// Moves the selected day by `days`.
    fn move_days(&mut self, days: i64, window: &mut Window, cx: &mut Context<Self>) {
        let date = self.model.read(cx).anchor() + TimeDelta::days(days);
        self.selected = None;
        self.go_to(date, window, cx);
    }

    /// Selects the next or previous event of the grid. With `same_day`, stays
    /// on the selected day and stops at its ends.
    fn cycle_event(&mut self, delta: isize, same_day: bool, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let anchor = model.anchor();
        let order = reading_order(&model.snapshot);
        let target = if same_day {
            let day: Vec<_> = order
                .iter()
                .filter(|(date, _)| *date == anchor)
                .cloned()
                .collect();
            let current = self
                .selected
                .as_ref()
                .and_then(|key| day.iter().position(|(_, k)| k == key));
            let ix = match current {
                Some(ix) => ix as isize + delta,
                None if delta >= 0 => 0,
                None => day.len() as isize - 1,
            };
            usize::try_from(ix).ok().and_then(|ix| day.get(ix).cloned())
        } else {
            neighbour(order, self.selected.as_ref(), anchor, delta).cloned()
        };
        let Some((date, key)) = target else {
            return;
        };
        let when = find_chip(&model.snapshot, &key).map(|chip| chip.when.clone());
        self.selected = Some(key.clone());
        self.model.update(cx, |m, cx| m.go_to(date, cx));
        // An open panel follows the selection.
        if self.detail.is_some()
            && let Some(when) = when
        {
            self.open_event_detail(key, when, cx);
        }
        self.after_move(cx);
    }

    fn close_detail(&mut self, _: &CloseDetail, _: &mut Window, cx: &mut Context<Self>) {
        if self.detail.is_none() && self.detail_load.is_none() {
            // Nothing to close: a second escape drops the grid's selection.
            self.selected = None;
        }
        self.detail_load = None;
        self.detail = None;
        self.detail_key = None;
        cx.notify();
    }

    fn previous_row(&mut self, _: &PreviousRow, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.read(cx).view() != ViewKind::Agenda {
            return self.step(-1, window, cx);
        }
        let list = self.list.read(cx);
        let at_top = list
            .selected_index()
            .is_none_or(|ix| list.delegate().is_first(ix));
        if at_top {
            self.model.update(cx, |m, cx| m.load_earlier(cx));
        } else {
            window.dispatch_action(Box::new(gpui_kit::base::actions::SelectUp), cx);
        }
    }

    fn jump_to_today(&mut self, _: &JumpToToday, window: &mut Window, cx: &mut Context<Self>) {
        self.go_today(window, cx);
    }

    fn sync_now(&mut self, _: &SyncNow, _: &mut Window, _: &mut Context<Self>) {
        let _ = self.commands.try_send(Command::SyncNow);
    }

    fn next_period(&mut self, _: &NextPeriod, window: &mut Window, cx: &mut Context<Self>) {
        self.step(1, window, cx);
    }

    fn previous_period(&mut self, _: &PreviousPeriod, window: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, window, cx);
    }

    fn next_day(&mut self, _: &NextDay, window: &mut Window, cx: &mut Context<Self>) {
        self.move_days(1, window, cx);
    }

    fn previous_day(&mut self, _: &PreviousDay, window: &mut Window, cx: &mut Context<Self>) {
        self.move_days(-1, window, cx);
    }

    /// Up and down move by a week in the month, and through the selected
    /// day's events in a time grid.
    fn grid_up(&mut self, _: &GridUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.read(cx).view() == ViewKind::Month {
            self.move_days(-7, window, cx);
        } else {
            self.cycle_event(-1, true, cx);
        }
    }

    fn grid_down(&mut self, _: &GridDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.read(cx).view() == ViewKind::Month {
            self.move_days(7, window, cx);
        } else {
            self.cycle_event(1, true, cx);
        }
    }

    fn next_event(&mut self, _: &NextEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.cycle_event(1, false, cx);
    }

    fn previous_event(&mut self, _: &PreviousEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.cycle_event(-1, false, cx);
    }

    /// Enter opens the selected event; with none selected, the selected day.
    fn open_selected(&mut self, _: &OpenSelected, window: &mut Window, cx: &mut Context<Self>) {
        let model = self.model.read(cx);
        let chip = self
            .selected
            .as_ref()
            .and_then(|key| find_chip(&model.snapshot, key));
        if let Some(chip) = chip {
            let (key, when) = (chip.key.clone(), chip.when.clone());
            self.open_event_detail(key, when, cx);
        } else if model.view() != ViewKind::Day {
            let date = model.anchor();
            self.open_day(date, window, cx);
        }
    }

    fn show_day(&mut self, _: &ShowDay, window: &mut Window, cx: &mut Context<Self>) {
        self.show_view(ViewKind::Day, window, cx);
    }

    fn show_week(&mut self, _: &ShowWeek, window: &mut Window, cx: &mut Context<Self>) {
        self.show_view(ViewKind::Week, window, cx);
    }

    fn show_month(&mut self, _: &ShowMonth, window: &mut Window, cx: &mut Context<Self>) {
        self.show_view(ViewKind::Month, window, cx);
    }

    fn show_agenda(&mut self, _: &ShowAgenda, window: &mut Window, cx: &mut Context<Self>) {
        self.show_view(ViewKind::Agenda, window, cx);
    }

    /// The day, week or month grid for the snapshot on screen.
    fn render_grid(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let model = self.model.read(cx);
        let snapshot = &model.snapshot;
        let now = Utc::now().with_timezone(&model.tz);
        if self.scroll_pending && snapshot.view.has_time_grid() && snapshot.range.is_some() {
            // An hour of context above the current time.
            let top = now.hour().saturating_sub(1) as f32 * time_grid::HOUR_HEIGHT;
            self.grid_scroll.set_offset(point(px(0.), px(-top)));
            self.scroll_pending = false;
        }
        let state = GridState {
            this: cx.entity().downgrade(),
            selected: self.selected.as_ref(),
            today: model.today(),
            anchor: model.anchor(),
        };
        if snapshot.view == ViewKind::Month {
            let height = f32::from(window.viewport_size().height)
                - f32::from(TITLE_BAR_HEIGHT)
                - TOOLBAR_HEIGHT
                - STATUS_BAR_HEIGHT;
            return month_grid::render(&snapshot.month, height, &state, cx);
        }
        time_grid::render(
            &snapshot.grid,
            &self.grid_scroll,
            Some((now.date_naive(), now.hour() * 60 + now.minute())),
            &state,
            cx,
        )
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        // The snapshot decides what is drawn, so a view switch never shows an
        // empty frame while its read runs.
        let shown = self.model.read(cx).snapshot.view;
        let body = if shown == ViewKind::Agenda {
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(List::new(&self.list))
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .track_focus(&self.grid_focus)
                .key_context(GRID_CONTEXT)
                .child(self.render_grid(window, cx))
                .into_any_element()
        };

        let model = self.model.read(cx);
        let mini = mini_calendar::render(
            &MiniCalendar {
                month: self.mini_month,
                week_start: model.week_start(),
                today: model.today(),
                anchor: model.anchor(),
                shown: (model.from, model.to),
            },
            &this,
            cx,
        );
        let title = view::title(model.view(), model.anchor(), model.from, model.to);
        let toggle = this.clone();
        v_flex()
            .size_full()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::close_detail))
            .on_action(cx.listener(Self::previous_row))
            .on_action(cx.listener(Self::jump_to_today))
            .on_action(cx.listener(Self::sync_now))
            .on_action(cx.listener(Self::next_period))
            .on_action(cx.listener(Self::previous_period))
            .on_action(cx.listener(Self::next_day))
            .on_action(cx.listener(Self::previous_day))
            .on_action(cx.listener(Self::grid_up))
            .on_action(cx.listener(Self::grid_down))
            .on_action(cx.listener(Self::next_event))
            .on_action(cx.listener(Self::previous_event))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::show_day))
            .on_action(cx.listener(Self::show_week))
            .on_action(cx.listener(Self::show_month))
            .on_action(cx.listener(Self::show_agenda))
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar::render(model, &self.commands, mini, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(toolbar::render(title, model.view(), &this, cx))
                            .child(body),
                    )
                    .when_some(self.detail.as_ref(), |row, detail| {
                        row.child(detail_panel::render(
                            detail,
                            self.show_raw,
                            move |_, cx| {
                                let _ = toggle.update(cx, |view, cx| {
                                    view.show_raw = !view.show_raw;
                                    cx.notify();
                                });
                            },
                            cx,
                        ))
                    }),
            )
            .child(status_bar::render(model, cx))
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: `gpui_kit::*` would shadow `#[test]` with GPUI's macro.
    use chrono::{Datelike as _, TimeDelta, Utc};
    use gpui_kit::component::IndexPath;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, point, px,
        size,
    };
    use serde_json::json;
    use takt_google::model::{CalendarListEntry, EventFields, Item};
    use takt_sync::Command;

    use super::MainView;
    use crate::agenda::RowKey;
    use crate::config::ViewSettings;
    use crate::model::PAGE_DAYS;
    use crate::ui::detail_panel::DetailContent;
    use crate::view::ViewKind;

    fn open_main_view(
        db: &std::path::Path,
        cx: &mut TestAppContext,
    ) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<MainView>) {
        let (window, view, _commands) = open_main_view_with_commands(db, cx);
        (window, view)
    }

    /// Opens on the agenda, in UTC.
    fn open_main_view_with_commands(
        db: &std::path::Path,
        cx: &mut TestAppContext,
    ) -> (
        gpui_kit::AnyWindowHandle,
        gpui_kit::Entity<MainView>,
        async_channel::Receiver<Command>,
    ) {
        open_in(db, ViewKind::Agenda, cx)
    }

    fn open_in(
        db: &std::path::Path,
        view: ViewKind,
        cx: &mut TestAppContext,
    ) -> (
        gpui_kit::AnyWindowHandle,
        gpui_kit::Entity<MainView>,
        async_channel::Receiver<Command>,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::keys::bind(cx);
        });
        let db = db.to_owned();
        let (commands, commands_rx) = async_channel::unbounded();
        let (_events_tx, events) = async_channel::unbounded();
        let (window, view) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(600.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let settings = ViewSettings {
                    default_view: view,
                    time_zone: Some(chrono_tz::UTC),
                    ..ViewSettings::default()
                };
                cx.new(|cx| MainView::new(db.clone(), settings, commands, events, window, cx))
            })
            .unwrap()
        });
        (window, view, commands_rx)
    }

    /// Opens a view over one seeded event, rendered, with its row selected.
    fn open_with_selected_event(
        cx: &mut TestAppContext,
    ) -> (
        tempfile::TempDir,
        gpui_kit::AnyWindowHandle,
        gpui_kit::Entity<MainView>,
        async_channel::Receiver<Command>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("takt.sqlite3");
        seed_one_event(&db);
        let (window, view, commands) = open_main_view_with_commands(&db, cx);
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let list = view.read(cx).list.clone();
            list.update(cx, |list, cx| {
                list.set_selected_index(Some(IndexPath::new(0)), window, cx)
            });
            window.render_frame(cx);
        })
        .unwrap();
        // Drain WindowFocus and anything else sent while opening.
        while commands.try_recv().is_ok() {}
        (dir, window, view, commands)
    }

    fn press(window: gpui_kit::AnyWindowHandle, key: &str, cx: &mut TestAppContext) {
        cx.update_window(window, |_, window, cx| {
            window.press(key, cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn detail_title(view: &gpui_kit::Entity<MainView>, cx: &mut TestAppContext) -> Option<String> {
        cx.read(|cx| match &view.read(cx).detail {
            Some(DetailContent::Event(d)) => Some(d.title.clone()),
            Some(DetailContent::Task { title, .. }) => Some(title.clone()),
            None => None,
        })
    }

    #[gpui_kit::test]
    fn enter_opens_details_raw_json_toggles_and_escape_closes(cx: &mut TestAppContext) {
        let (_dir, window, view, _commands) = open_with_selected_event(cx);
        press(window, "enter", cx);
        assert_eq!(detail_title(&view, cx).as_deref(), Some("Soon"));
        assert!(!cx.read(|cx| view.read(cx).show_raw));

        cx.update_window(window, |_, window, cx| {
            window.click("toggle-raw", cx);
            window.render_frame(cx);
        })
        .unwrap();
        assert!(cx.read(|cx| view.read(cx).show_raw));

        press(window, "escape", cx);
        assert_eq!(detail_title(&view, cx), None);
    }

    #[gpui_kit::test]
    fn k_on_the_first_row_loads_earlier_days_and_t_returns(cx: &mut TestAppContext) {
        let (_dir, window, view, _commands) = open_with_selected_event(cx);
        let model = cx.read(|cx| view.read(cx).model().clone());
        let from = cx.read(|cx| model.read(cx).from);

        press(window, "k", cx);
        assert_eq!(
            cx.read(|cx| model.read(cx).from),
            from - TimeDelta::days(PAGE_DAYS)
        );

        press(window, "t", cx);
        let (from_now, today) = cx.read(|cx| (model.read(cx).from, model.read(cx).today()));
        assert_eq!(from_now, today);
    }

    #[gpui_kit::test]
    fn k_below_the_first_row_moves_up(cx: &mut TestAppContext) {
        let (_dir, window, view, _commands) = open_with_selected_event(cx);
        let model = cx.read(|cx| view.read(cx).model().clone());
        // A second event two hours after the first.
        {
            let db = cx.read(|cx| model.read(cx).db().to_owned());
            let mut store = takt_store::Store::open(&db).unwrap();
            let start = Utc::now() + TimeDelta::hours(3);
            let event: Item<EventFields> = Item::from_value(json!({
                "id": "e2", "summary": "Later",
                "start": {"dateTime": start.to_rfc3339()},
                "end": {"dateTime": (start + TimeDelta::hours(1)).to_rfc3339()}
            }))
            .unwrap();
            store
                .apply_events_page("acc", "cal", &[event], None)
                .unwrap();
        }
        model.update(cx, |m, cx| m.reload(cx));
        cx.run_until_parked();
        let from = cx.read(|cx| model.read(cx).from);
        let last = cx.read(|cx| {
            let sections = &view.read(cx).list.read(cx).delegate().sections;
            let s = sections.len() - 1;
            IndexPath::new(sections[s].rows.len() - 1).section(s)
        });
        cx.update_window(window, |_, window, cx| {
            let list = view.read(cx).list.clone();
            list.update(cx, |list, cx| {
                list.set_selected_index(Some(last), window, cx)
            });
            window.render_frame(cx);
        })
        .unwrap();

        press(window, "k", cx);
        let selected = cx.read(|cx| view.read(cx).list.read(cx).selected_index());
        assert_eq!(selected.map(|ix| (ix.section, ix.row)), Some((0, 0)));
        assert_eq!(cx.read(|cx| model.read(cx).from), from);
    }

    #[gpui_kit::test]
    fn secondary_r_asks_sync_to_run_now(cx: &mut TestAppContext) {
        let (_dir, window, _view, commands) = open_with_selected_event(cx);
        press(window, "secondary-r", cx);
        assert_eq!(commands.try_recv(), Ok(Command::SyncNow));
    }

    #[cfg(not(target_os = "macos"))]
    #[gpui_kit::test]
    fn ctrl_r_asks_sync_to_run_now(cx: &mut TestAppContext) {
        let (_dir, window, _view, commands) = open_with_selected_event(cx);
        press(window, "ctrl-r", cx);
        assert_eq!(commands.try_recv(), Ok(Command::SyncNow));
    }

    #[gpui_kit::test]
    fn escape_keeps_the_selection_and_k_then_moves_up(cx: &mut TestAppContext) {
        let (_dir, window, view, _commands) = open_with_selected_event(cx);
        let model = cx.read(|cx| view.read(cx).model().clone());
        {
            let db = cx.read(|cx| model.read(cx).db().to_owned());
            let mut store = takt_store::Store::open(&db).unwrap();
            let start = Utc::now() + TimeDelta::hours(3);
            let event: Item<EventFields> = Item::from_value(json!({
                "id": "e2", "summary": "Later",
                "start": {"dateTime": start.to_rfc3339()},
                "end": {"dateTime": (start + TimeDelta::hours(1)).to_rfc3339()}
            }))
            .unwrap();
            store
                .apply_events_page("acc", "cal", &[event], None)
                .unwrap();
        }
        model.update(cx, |m, cx| m.reload(cx));
        cx.run_until_parked();
        let from = cx.read(|cx| model.read(cx).from);
        let last = cx.read(|cx| {
            let sections = &view.read(cx).list.read(cx).delegate().sections;
            let s = sections.len() - 1;
            IndexPath::new(sections[s].rows.len() - 1).section(s)
        });
        cx.update_window(window, |_, window, cx| {
            let list = view.read(cx).list.clone();
            list.update(cx, |list, cx| {
                list.set_selected_index(Some(last), window, cx)
            });
            window.render_frame(cx);
        })
        .unwrap();

        press(window, "enter", cx);
        assert_eq!(detail_title(&view, cx).as_deref(), Some("Later"));
        press(window, "escape", cx);
        assert_eq!(detail_title(&view, cx), None);
        let selected = cx.read(|cx| view.read(cx).list.read(cx).selected_index());
        assert_eq!(selected, Some(last));

        press(window, "k", cx);
        let selected = cx.read(|cx| view.read(cx).list.read(cx).selected_index());
        assert_eq!(selected.map(|ix| (ix.section, ix.row)), Some((0, 0)));
        assert_eq!(cx.read(|cx| model.read(cx).from), from);
    }

    #[gpui_kit::test]
    fn remove_asks_first_then_sends_remove_account(cx: &mut TestAppContext) {
        let (_dir, window, _view, commands) = open_with_selected_event(cx);
        cx.update_window(window, |_, window, cx| {
            window.click("remove-acc", cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert!(
            commands.try_recv().is_err(),
            "nothing is sent before confirming"
        );

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("ok", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            commands.try_recv(),
            Ok(Command::RemoveAccount {
                account_id: "acc".into()
            })
        );
    }

    #[gpui_kit::test]
    fn add_account_sends_add_account(cx: &mut TestAppContext) {
        let (_dir, window, _view, commands) = open_with_selected_event(cx);
        cx.update_window(window, |_, window, cx| window.click("add-account", cx))
            .unwrap();
        assert_eq!(commands.try_recv(), Ok(Command::AddAccount));
    }

    /// Seeds one event an hour from now.
    fn seed_one_event(db: &std::path::Path) {
        let mut store = takt_store::Store::open(db).unwrap();
        store.upsert_account("acc", "me@example.com").unwrap();
        let cal: Item<CalendarListEntry> =
            Item::from_value(json!({"id": "cal", "selected": true})).unwrap();
        store.apply_calendar_list_page("acc", &[cal], None).unwrap();
        let start = Utc::now() + TimeDelta::hours(1);
        let event: Item<EventFields> = Item::from_value(json!({
            "id": "e", "summary": "Soon",
            "start": {"dateTime": start.to_rfc3339()},
            "end": {"dateTime": (start + TimeDelta::hours(1)).to_rfc3339()}
        }))
        .unwrap();
        store
            .apply_events_page("acc", "cal", &[event], None)
            .unwrap();
    }

    #[gpui_kit::test]
    fn load_earlier_is_reachable_with_an_empty_agenda(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("takt.sqlite3");
        takt_store::Store::open(&db).unwrap();
        let (window, view) = open_main_view(&db, cx);
        cx.run_until_parked();
        let model = cx.read(|cx| view.read(cx).model().clone());
        let from = cx.read(|cx| model.read(cx).from);

        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("load-earlier", cx);
        })
        .unwrap();
        cx.run_until_parked();

        assert_eq!(
            cx.read(|cx| model.read(cx).from),
            from - TimeDelta::days(PAGE_DAYS)
        );
    }

    /// The list asks for more whenever its end is near the bottom; with few
    /// events that used to extend the range forever.
    #[gpui_kit::test]
    fn scrolling_does_not_extend_a_sparse_agenda_forever(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("takt.sqlite3");
        seed_one_event(&db);
        let (window, view) = open_main_view(&db, cx);
        cx.run_until_parked();
        for _ in 0..20 {
            cx.update_window(window, |_, window, cx| window.render_frame(cx))
                .unwrap();
            cx.run_until_parked();
        }
        let model = cx.read(|cx| view.read(cx).model().clone());
        let (from, to, rows) = cx.read(|cx| {
            let m = model.read(cx);
            (m.from, m.to, m.snapshot.sections.len())
        });
        assert_eq!(rows, 1);
        assert!(
            (to - from).num_days() <= 2 * PAGE_DAYS,
            "range grew to {} days",
            (to - from).num_days()
        );

        // "Load later" still goes further by hand.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("load-later", cx);
        })
        .unwrap();
        cx.run_until_parked();
        assert_eq!(
            cx.read(|cx| model.read(cx).to),
            to + TimeDelta::days(PAGE_DAYS)
        );
    }

    // ---- Day, week and month grids ----

    fn id(text: String) -> gpui_kit::SharedString {
        gpui_kit::SharedString::from(text)
    }

    fn timed(id: &str, title: &str, day: chrono::NaiveDate, hour: u32) -> Item<EventFields> {
        let start = day.and_hms_opt(hour, 0, 0).unwrap().and_utc();
        Item::from_value(json!({
            "id": id, "summary": title,
            "start": {"dateTime": start.to_rfc3339()},
            "end": {"dateTime": (start + TimeDelta::hours(1)).to_rfc3339()}
        }))
        .unwrap()
    }

    /// Seeds "Holiday" all day today and "Lunch" at 12:00 UTC today, plus `extra`.
    fn seed_today(db: &std::path::Path, extra: &[Item<EventFields>]) -> chrono::NaiveDate {
        let today = Utc::now().date_naive();
        let mut store = takt_store::Store::open(db).unwrap();
        store.upsert_account("acc", "me@example.com").unwrap();
        let cal: Item<CalendarListEntry> =
            Item::from_value(json!({"id": "cal", "summary": "Me", "selected": true})).unwrap();
        store.apply_calendar_list_page("acc", &[cal], None).unwrap();
        let holiday: Item<EventFields> = Item::from_value(json!({
            "id": "holiday", "summary": "Holiday",
            "start": {"date": today.to_string()},
            "end": {"date": (today + TimeDelta::days(1)).to_string()}
        }))
        .unwrap();
        let mut events = vec![holiday, timed("lunch", "Lunch", today, 12)];
        events.extend(extra.iter().cloned());
        store
            .apply_events_page("acc", "cal", &events, None)
            .unwrap();
        today
    }

    struct Grid {
        _dir: tempfile::TempDir,
        window: gpui_kit::AnyWindowHandle,
        view: gpui_kit::Entity<MainView>,
        commands: async_channel::Receiver<Command>,
        today: chrono::NaiveDate,
    }

    impl Grid {
        /// Opens `view` over today's seeded events, drawn, with the time grid
        /// scrolled to 11:00 so "Lunch" is on screen.
        fn open(view: ViewKind, extra: &[Item<EventFields>], cx: &mut TestAppContext) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let db = dir.path().join("takt.sqlite3");
            let today = seed_today(&db, extra);
            let (window, main, commands) = open_in(&db, view, cx);
            cx.run_until_parked();
            let grid = Self {
                _dir: dir,
                window,
                view: main,
                commands,
                today,
            };
            grid.frame(cx);
            grid.scroll_to_late_morning(cx);
            while grid.commands.try_recv().is_ok() {}
            grid
        }

        fn frame(&self, cx: &mut TestAppContext) {
            cx.update_window(self.window, |_, window, cx| window.render_frame(cx))
                .unwrap();
            cx.run_until_parked();
        }

        fn scroll_to_late_morning(&self, cx: &mut TestAppContext) {
            let view = self.view.clone();
            cx.update_window(self.window, |_, window, cx| {
                let scroll = view.read(cx).grid_scroll.clone();
                scroll.set_offset(point(px(0.), px(-11. * crate::ui::time_grid::HOUR_HEIGHT)));
                window.refresh();
                window.render_frame(cx);
            })
            .unwrap();
        }

        fn click(&self, target: String, cx: &mut TestAppContext) {
            cx.update_window(self.window, |_, window, cx| {
                window.click(id(target), cx);
                window.render_frame(cx);
            })
            .unwrap();
            cx.run_until_parked();
            self.frame(cx);
        }

        fn press(&self, key: &str, cx: &mut TestAppContext) {
            press(self.window, key, cx);
            self.frame(cx);
        }

        fn shown(&self, cx: &mut TestAppContext) -> (ViewKind, chrono::NaiveDate) {
            cx.read(|cx| {
                let model = self.view.read(cx).model().read(cx);
                assert_eq!(model.snapshot.view, model.view(), "the read has landed");
                (model.view(), model.anchor())
            })
        }

        fn selected(&self, cx: &mut TestAppContext) -> Option<String> {
            cx.read(|cx| match self.view.read(cx).selected_event() {
                Some(RowKey::Event { event_id, .. }) => Some(event_id.clone()),
                _ => None,
            })
        }
    }

    #[gpui_kit::test]
    fn clicking_a_block_selects_it_and_opens_its_details(cx: &mut TestAppContext) {
        let grid = Grid::open(ViewKind::Week, &[], cx);
        let today = grid.today;
        grid.click(format!("block-acc-cal-lunch-{today}"), cx);
        assert_eq!(detail_title(&grid.view, cx).as_deref(), Some("Lunch"));
        assert_eq!(grid.selected(cx).as_deref(), Some("lunch"));

        // The all-day strip's bar is keyed by the row's first day.
        let row = cx.read(|cx| grid.view.read(cx).model().read(cx).from);
        grid.click(format!("bar-acc-cal-holiday-{row}"), cx);
        assert_eq!(detail_title(&grid.view, cx).as_deref(), Some("Holiday"));

        // Escape closes the panel first, then drops the selection.
        grid.press("escape", cx);
        assert_eq!(detail_title(&grid.view, cx), None);
        assert_eq!(grid.selected(cx).as_deref(), Some("holiday"));
        grid.press("escape", cx);
        assert_eq!(grid.selected(cx), None);
    }

    #[gpui_kit::test]
    fn letter_and_number_keys_switch_views(cx: &mut TestAppContext) {
        let grid = Grid::open(ViewKind::Week, &[], cx);
        let today = grid.today;
        for (key, view) in [
            ("m", ViewKind::Month),
            ("d", ViewKind::Day),
            ("a", ViewKind::Agenda),
            ("w", ViewKind::Week),
            ("1", ViewKind::Day),
            ("3", ViewKind::Month),
            ("4", ViewKind::Agenda),
            ("2", ViewKind::Week),
        ] {
            grid.press(key, cx);
            assert_eq!(grid.shown(cx), (view, today), "after {key}");
        }
    }

    #[gpui_kit::test]
    fn keys_step_periods_and_days(cx: &mut TestAppContext) {
        let grid = Grid::open(ViewKind::Week, &[], cx);
        let today = grid.today;
        let day = |n: i64| today + TimeDelta::days(n);
        for (key, expected) in [
            ("j", day(7)),
            ("k", today),
            ("n", day(7)),
            ("p", today),
            ("pagedown", day(7)),
            ("pageup", today),
            ("right", day(1)),
            ("l", day(2)),
            ("left", day(1)),
            ("h", today),
        ] {
            grid.press(key, cx);
            assert_eq!(grid.shown(cx), (ViewKind::Week, expected), "after {key}");
        }

        grid.press("j", cx);
        grid.press("t", cx);
        assert_eq!(grid.shown(cx), (ViewKind::Week, today));

        // In the month, up and down move a week.
        grid.press("m", cx);
        grid.press("down", cx);
        assert_eq!(grid.shown(cx), (ViewKind::Month, day(7)));
        grid.press("up", cx);
        assert_eq!(grid.shown(cx), (ViewKind::Month, today));

        // In the day view a period is one day.
        grid.press("d", cx);
        grid.press("j", cx);
        assert_eq!(grid.shown(cx), (ViewKind::Day, day(1)));
    }

    #[gpui_kit::test]
    fn tab_walks_the_events_and_enter_opens_the_selected_one(cx: &mut TestAppContext) {
        let grid = Grid::open(ViewKind::Week, &[], cx);
        grid.press("tab", cx);
        assert_eq!(grid.selected(cx).as_deref(), Some("holiday"));
        grid.press("tab", cx);
        assert_eq!(grid.selected(cx).as_deref(), Some("lunch"));
        grid.press("tab", cx);
        assert_eq!(
            grid.selected(cx).as_deref(),
            Some("holiday"),
            "wraps around"
        );
        grid.press("shift-tab", cx);
        assert_eq!(grid.selected(cx).as_deref(), Some("lunch"));
        assert_eq!(
            detail_title(&grid.view, cx),
            None,
            "selecting opens nothing"
        );

        grid.press("enter", cx);
        assert_eq!(detail_title(&grid.view, cx).as_deref(), Some("Lunch"));
        // An open panel follows the selection.
        grid.press("up", cx);
        assert_eq!(grid.selected(cx).as_deref(), Some("holiday"));
        assert_eq!(detail_title(&grid.view, cx).as_deref(), Some("Holiday"));
        grid.press("up", cx);
        assert_eq!(
            grid.selected(cx).as_deref(),
            Some("holiday"),
            "stops at the top"
        );
        grid.press("down", cx);
        assert_eq!(detail_title(&grid.view, cx).as_deref(), Some("Lunch"));
    }

    #[gpui_kit::test]
    fn enter_without_a_selection_opens_the_selected_day(cx: &mut TestAppContext) {
        let grid = Grid::open(ViewKind::Month, &[], cx);
        grid.press("right", cx);
        grid.press("enter", cx);
        assert_eq!(
            grid.shown(cx),
            (ViewKind::Day, grid.today + TimeDelta::days(1))
        );
    }

    #[gpui_kit::test]
    fn clicking_a_calendar_asks_to_hide_it(cx: &mut TestAppContext) {
        let grid = Grid::open(ViewKind::Week, &[], cx);
        grid.click("calendar-acc-cal".into(), cx);
        assert_eq!(
            grid.commands.try_recv(),
            Ok(Command::SetCalendarVisible {
                account_id: "acc".into(),
                calendar_id: "cal".into(),
                visible: false,
            })
        );
    }

    #[gpui_kit::test]
    fn toolbar_and_mini_calendar_move_the_view(cx: &mut TestAppContext) {
        let grid = Grid::open(ViewKind::Week, &[], cx);
        let today = grid.today;
        grid.click("next".into(), cx);
        assert_eq!(grid.shown(cx), (ViewKind::Week, today + TimeDelta::days(7)));
        grid.click("prev".into(), cx);
        grid.click("prev".into(), cx);
        assert_eq!(grid.shown(cx), (ViewKind::Week, today - TimeDelta::days(7)));
        grid.click("today".into(), cx);
        assert_eq!(grid.shown(cx), (ViewKind::Week, today));

        grid.click("view-month".into(), cx);
        assert_eq!(grid.shown(cx), (ViewKind::Month, today));
        grid.click("view-day".into(), cx);
        assert_eq!(grid.shown(cx), (ViewKind::Day, today));

        // The mini calendar jumps to a day and keeps the view.
        let first = today.with_day(1).unwrap();
        grid.click(format!("mini-{first}"), cx);
        assert_eq!(grid.shown(cx), (ViewKind::Day, first));

        // Its arrows page the mini month only.
        grid.click("mini-next".into(), cx);
        let next_month = first.checked_add_months(chrono::Months::new(1)).unwrap();
        assert_eq!(cx.read(|cx| grid.view.read(cx).mini_month), next_month);
        assert_eq!(grid.shown(cx), (ViewKind::Day, first));
        grid.click(format!("mini-{next_month}"), cx);
        assert_eq!(grid.shown(cx), (ViewKind::Day, next_month));
    }

    #[gpui_kit::test]
    fn a_week_header_opens_its_day(cx: &mut TestAppContext) {
        let grid = Grid::open(ViewKind::Week, &[], cx);
        let from = cx.read(|cx| grid.view.read(cx).model().read(cx).from);
        grid.click(format!("head-{from}"), cx);
        assert_eq!(grid.shown(cx), (ViewKind::Day, from));
    }

    #[gpui_kit::test]
    fn a_full_month_cell_says_how_many_more_and_opens_the_day(cx: &mut TestAppContext) {
        let today = Utc::now().date_naive();
        let extra: Vec<_> = (0..10)
            .map(|n| timed(&format!("extra{n}"), "Extra", today, 8 + n))
            .collect();
        let grid = Grid::open(ViewKind::Month, &extra, cx);
        cx.read(|cx| {
            let month = &grid.view.read(cx).model().read(cx).snapshot.month;
            let week = month
                .weeks
                .iter()
                .find(|week| week.days.contains(&today))
                .unwrap();
            assert_eq!(week.bars.len(), 12);
            assert_eq!(month.order.len(), 12);
        });

        // Selecting a day keeps the month; its number opens the day.
        let first = today.with_day(1).unwrap();
        grid.click(format!("day-{first}"), cx);
        assert_eq!(grid.shown(cx), (ViewKind::Month, first));
        grid.click(format!("more-{today}"), cx);
        assert_eq!(grid.shown(cx), (ViewKind::Day, today));

        grid.press("m", cx);
        grid.click(format!("num-{first}"), cx);
        assert_eq!(grid.shown(cx), (ViewKind::Day, first));
    }

    #[gpui_kit::test]
    fn a_hidden_calendars_events_leave_the_grid(cx: &mut TestAppContext) {
        let grid = Grid::open(ViewKind::Week, &[], cx);
        let model = cx.read(|cx| grid.view.read(cx).model().clone());
        assert_eq!(cx.read(|cx| model.read(cx).snapshot.grid.order.len()), 2);
        grid.press("tab", cx);
        assert!(grid.selected(cx).is_some());

        // What the sync thread does for SetCalendarVisible.
        let changes = {
            let db = cx.read(|cx| model.read(cx).db().to_owned());
            takt_store::Store::open(&db)
                .unwrap()
                .set_calendar_visible("acc", "cal", Some(false))
                .unwrap()
        };
        model.update(cx, |m, cx| {
            m.handle_sync_event(
                takt_sync::SyncEvent::Changed {
                    account_id: "acc".into(),
                    changes,
                },
                cx,
            )
        });
        cx.run_until_parked();
        grid.frame(cx);
        cx.read(|cx| {
            let snapshot = &model.read(cx).snapshot;
            assert!(snapshot.grid.order.is_empty());
            assert!(!snapshot.calendars[0].visible);
        });
        assert_eq!(grid.selected(cx), None, "its selection goes with it");
    }
}
