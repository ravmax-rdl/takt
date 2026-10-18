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
    use chrono::TimeDelta;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
    };

    use super::MainView;
    use crate::model::PAGE_DAYS;

    #[gpui_kit::test]
    fn load_earlier_is_reachable_with_an_empty_agenda(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("takt.sqlite3");
        takt_store::Store::open(&db).unwrap();
        let (commands, _commands_rx) = async_channel::unbounded();
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
}
