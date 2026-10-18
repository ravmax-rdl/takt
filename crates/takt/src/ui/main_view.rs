use std::path::PathBuf;

use async_channel::{Receiver, Sender};
use chrono::Utc;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::list::{List, ListEvent, ListState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;
use takt_sync::{Command, SyncEvent};

use crate::model::{CalendarModel, ModelEvent};
use crate::ui::agenda_list::AgendaDelegate;

pub struct MainView {
    model: Entity<CalendarModel>,
    list: Entity<ListState<AgendaDelegate>>,
    commands: Sender<Command>,
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
            cx.observe_window_activation(window, |this, window, _| {
                let _ = this
                    .commands
                    .try_send(Command::WindowFocus(window.is_window_active()));
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

        list.update(cx, |list, cx| list.focus(window, cx));
        Self {
            model,
            list,
            commands,
            _subscriptions: subscriptions,
            _tasks: vec![bridge],
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

    fn on_list_event(&mut self, _event: &ListEvent, _window: &mut Window, _cx: &mut Context<Self>) {
    }
}

impl Render for MainView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().h_full().child(List::new(&self.list))),
            )
            .text_color(cx.theme().foreground)
    }
}

#[cfg(test)]
mod tests {
    // Explicit imports: `gpui_kit::*` would shadow `#[test]` with GPUI's macro.
    use chrono::{TimeDelta, Utc};
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
    };
    use serde_json::json;
    use takt_google::model::{CalendarListEntry, EventFields, Item};

    use super::MainView;
    use crate::model::PAGE_DAYS;

    fn open_main_view(
        db: &std::path::Path,
        cx: &mut TestAppContext,
    ) -> (gpui_kit::AnyWindowHandle, gpui_kit::Entity<MainView>) {
        cx.update(gpui_kit::init);
        let db = db.to_owned();
        let (commands, _commands_rx) = async_channel::unbounded();
        let (_events_tx, events) = async_channel::unbounded();
        cx.update(|cx| {
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
        })
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
