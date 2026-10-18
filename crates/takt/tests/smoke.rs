//! Opens the main window headless over a seeded cache and renders it.

use chrono::{TimeDelta, Utc};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Point, TestAppContext, WindowBounds, WindowOptions, px, size,
};
use serde_json::json;
use takt_google::model::{CalendarListEntry, EventFields, Item};
use takt_store::Store;

fn seed(db: &std::path::Path) {
    let mut store = Store::open(db).unwrap();
    store.upsert_account("acc", "me@example.com").unwrap();
    let cal: Item<CalendarListEntry> =
        Item::from_value(json!({"id": "cal", "summary": "Me", "selected": true})).unwrap();
    store.apply_calendar_list_page("acc", &[cal], None).unwrap();
    let start = Utc::now() + TimeDelta::hours(1);
    let event: Item<EventFields> = Item::from_value(json!({
        "id": "e", "summary": "Smoke test",
        "start": {"dateTime": start.to_rfc3339()},
        "end": {"dateTime": (start + TimeDelta::hours(1)).to_rfc3339()}
    }))
    .unwrap();
    store
        .apply_events_page("acc", "cal", &[event], None)
        .unwrap();
}

#[gpui_kit::test]
fn main_view_renders_the_seeded_agenda(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("takt.sqlite3");
    seed(&db);
    cx.update(|cx| {
        gpui_kit::init(cx);
        takt::keys::bind(cx);
    });
    let (commands, _command_rx) = async_channel::unbounded();
    let (_event_tx, events) = async_channel::unbounded();
    // gpui-kit's open_window wraps the view in its Root, which the theme needs.
    let (window, view) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(800.), px(600.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| takt::ui::MainView::new(db.clone(), commands, events, window, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| window.render_frame(cx))
        .unwrap();

    let titles: Vec<String> = cx.read(|cx| {
        view.read(cx)
            .model()
            .read(cx)
            .snapshot
            .sections
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| r.title.clone()))
            .collect()
    });
    assert!(titles.contains(&"Smoke test".to_owned()), "{titles:?}");
}
