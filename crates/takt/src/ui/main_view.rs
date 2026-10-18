use std::path::PathBuf;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use chrono::Utc;
use gpui_kit::component::list::{List, ListEvent, ListState};
use gpui_kit::component::{ActiveTheme as _, IndexPath, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use takt_sync::{Command, SyncEvent};

use crate::agenda::RowKey;
use crate::keys::{CONTEXT, CloseDetail, JumpToToday, PreviousRow, SyncNow};
use crate::model::{CalendarModel, ModelEvent};
use crate::ui::agenda_list::AgendaDelegate;
use crate::ui::detail_panel::{self, DetailContent};
use crate::ui::{sidebar, status_bar};

/// How often the status bar's "synced 12s ago" texts are redrawn.
const STATUS_TICK: Duration = Duration::from_secs(15);

pub struct MainView {
    model: Entity<CalendarModel>,
    list: Entity<ListState<AgendaDelegate>>,
    commands: Sender<Command>,
    detail: Option<DetailContent>,
    show_raw: bool,
    /// The read behind the detail being opened; a newer open replaces (cancels) it.
    detail_load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    _tasks: Vec<Task<()>>,
}

impl MainView {
    pub fn new(
        db: PathBuf,
        commands: Sender<Command>,
        events: Receiver<SyncEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tz = crate::display_zone();
        let today = Utc::now().with_timezone(&tz).date_naive();
        let model = cx.new(|cx| CalendarModel::new(db, tz, today, cx));
        let delegate = AgendaDelegate {
            model: model.clone(),
            sections: Vec::new(),
            selected: None,
        };
        let list = cx.new(|cx| ListState::new(delegate, window, cx).selectable(true));

        let subscriptions = vec![
            cx.subscribe_in(&model, window, |this, _, _: &ModelEvent, window, cx| {
                this.refresh_list(window, cx)
            }),
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

        // Redraws the relative sync times, and moves "Today" on past midnight
        // while the window stays focused.
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

        list.update(cx, |list, cx| list.focus(window, cx));
        Self {
            model,
            list,
            commands,
            detail: None,
            show_raw: false,
            detail_load: None,
            _subscriptions: subscriptions,
            _tasks: vec![bridge, ticker],
        }
    }

    pub fn model(&self) -> &Entity<CalendarModel> {
        &self.model
    }

    /// Copies new sections into the list and keeps the same row, on the same
    /// day, selected.
    fn refresh_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sections = self.model.read(cx).snapshot.sections.clone();
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

    fn on_list_event(&mut self, event: &ListEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            ListEvent::Confirm(ix) => self.open_detail(*ix, cx),
            ListEvent::Cancel => self.close_detail(&CloseDetail, window, cx),
            ListEvent::Select(_) => {}
        }
    }

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
        match row.key.clone() {
            RowKey::Task { .. } => {
                self.detail_load = None;
                self.detail = Some(DetailContent::Task {
                    title: row.title.clone(),
                    when,
                    list: row.location.clone().unwrap_or_default(),
                    completed: row.completed,
                });
                self.show_raw = false;
                cx.notify();
            }
            RowKey::Event {
                account_id,
                calendar_id,
                event_id,
                ..
            } => {
                let model = self.model.read(cx);
                let db = model.db().to_path_buf();
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
                    takt_store::Store::open_read_only(&db)?.event_raw(
                        &account_id,
                        &calendar_id,
                        &event_id,
                    )
                });
                self.detail_load = Some(cx.spawn(async move |this, cx| {
                    let raw = read.await;
                    let _ = this.update(cx, |view, cx| {
                        view.detail_load = None;
                        match raw {
                            Ok(Some(raw)) => {
                                view.detail = Some(DetailContent::Event(Box::new(
                                    crate::detail::event_detail(&raw, when, calendar, account),
                                )));
                                view.show_raw = false;
                            }
                            Ok(None) => view.detail = None,
                            Err(error) => tracing::error!(%error, "cannot read event details"),
                        }
                        cx.notify();
                    });
                }));
            }
        }
    }

    fn close_detail(&mut self, _: &CloseDetail, _: &mut Window, cx: &mut Context<Self>) {
        self.detail_load = None;
        self.detail = None;
        cx.notify();
    }

    fn previous_row(&mut self, _: &PreviousRow, window: &mut Window, cx: &mut Context<Self>) {
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

    fn jump_to_today(&mut self, _: &JumpToToday, _: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |m, cx| m.jump_to_today(cx));
    }

    fn sync_now(&mut self, _: &SyncNow, _: &mut Window, _: &mut Context<Self>) {
        let _ = self.commands.try_send(Command::SyncNow);
    }
}

impl Render for MainView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let this = cx.entity().downgrade();
        v_flex()
            .size_full()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::close_detail))
            .on_action(cx.listener(Self::previous_row))
            .on_action(cx.listener(Self::jump_to_today))
            .on_action(cx.listener(Self::sync_now))
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar::render(model, &self.commands, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(List::new(&self.list)),
                    )
                    .when_some(self.detail.as_ref(), |row, detail| {
                        row.child(detail_panel::render(
                            detail,
                            self.show_raw,
                            move |_, cx| {
                                let _ = this.update(cx, |view, cx| {
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
    use chrono::{TimeDelta, Utc};
    use gpui_kit::component::IndexPath;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
    };
    use serde_json::json;
    use takt_google::model::{CalendarListEntry, EventFields, Item};
    use takt_sync::Command;

    use super::MainView;
    use crate::model::PAGE_DAYS;
    use crate::ui::detail_panel::DetailContent;

    fn open_main_view(
        db: &std::path::Path,
        cx: &mut TestAppContext,
    ) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<MainView>) {
        let (window, view, _commands) = open_main_view_with_commands(db, cx);
        (window, view)
    }

    fn open_main_view_with_commands(
        db: &std::path::Path,
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
                cx.new(|cx| MainView::new(db.clone(), commands, events, window, cx))
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
}
