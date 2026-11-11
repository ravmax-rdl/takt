//! Opens takt's main view over made-up events, with no Google account and no
//! sync. For looking at the views while working on them:
//!
//! ```text
//! cargo run -p takt --example demo -- month
//! ```
//!
//! The argument is the view to open in: day, week (the default), month or agenda.

use chrono::{Datelike as _, NaiveDate, TimeDelta, Utc};
use gpui_kit::component::{ActiveTheme as _, Theme, TitleBar, v_flex};
use gpui_kit::*;
use serde_json::{Value, json};
use takt::config::ViewSettings;
use takt::ui::MainView;
use takt::view::ViewKind;
use takt_google::model::{CalendarListEntry, EventFields, Item, Setting};
use takt_store::Store;

struct Demo {
    main: Entity<MainView>,
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(TitleBar::new().child("takt demo"))
            .child(div().flex_1().min_h_0().child(self.main.clone()))
    }
}

fn timed(id: &str, title: &str, day: NaiveDate, start: (u32, u32), minutes: i64) -> Value {
    let from = day.and_hms_opt(start.0, start.1, 0).unwrap().and_utc();
    json!({
        "id": id, "summary": title,
        "start": {"dateTime": from.to_rfc3339(), "timeZone": "UTC"},
        "end": {"dateTime": (from + TimeDelta::minutes(minutes)).to_rfc3339(), "timeZone": "UTC"}
    })
}

fn all_day(id: &str, title: &str, day: NaiveDate, days: i64) -> Value {
    json!({
        "id": id, "summary": title,
        "start": {"date": day.to_string()},
        "end": {"date": (day + TimeDelta::days(days)).to_string()}
    })
}

fn seed(db: &std::path::Path) {
    let mut store = Store::open(db).unwrap();
    let today = Utc::now().date_naive();
    let monday = today - TimeDelta::days(i64::from(today.weekday().num_days_from_monday()));
    let day = |n: i64| monday + TimeDelta::days(n);
    let items = |values: Vec<Value>| -> Vec<Item<EventFields>> {
        values
            .into_iter()
            .map(|v| Item::from_value(v).unwrap())
            .collect()
    };
    let calendar = |v: Value| -> Item<CalendarListEntry> { Item::from_value(v).unwrap() };

    store.upsert_account("home", "you@example.com").unwrap();
    store.upsert_account("work", "you@work.example").unwrap();
    let settings: Vec<Item<Setting>> = vec![
        Item::from_value(json!({"id": "weekStart", "value": "1"})).unwrap(),
        Item::from_value(json!({"id": "format24HourTime", "value": "true"})).unwrap(),
    ];
    store.put_settings("home", &settings).unwrap();
    store
        .apply_calendar_list_page(
            "home",
            &[
                calendar(
                    json!({"id": "personal", "summary": "Personal", "primary": true,
                    "selected": true, "backgroundColor": "#7986cb"}),
                ),
                calendar(
                    json!({"id": "family", "summary": "Family", "selected": true,
                    "backgroundColor": "#33b679"}),
                ),
                calendar(
                    json!({"id": "birthdays", "summary": "Birthdays", "selected": false,
                    "backgroundColor": "#f6bf26"}),
                ),
            ],
            None,
        )
        .unwrap();
    store
        .apply_calendar_list_page(
            "work",
            &[calendar(
                json!({"id": "work", "summary": "Work", "primary": true,
                "selected": true, "backgroundColor": "#e67c73"}),
            )],
            None,
        )
        .unwrap();

    let mut weekly = timed("standup", "Standup", day(-28), (9, 0), 15);
    weekly["recurrence"] = json!(["RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"]);
    let mut declined = timed("optional", "Optional sync", day(3), (15, 0), 60);
    declined["attendees"] = json!([{"email": "you@work.example", "self": true,
        "responseStatus": "declined"}]);
    let mut abroad = timed("call", "Call with New York", day(2), (15, 0), 45);
    abroad["start"]["timeZone"] = json!("America/New_York");
    abroad["end"]["timeZone"] = json!("America/New_York");
    store
        .apply_events_page(
            "work",
            "work",
            &items(vec![
                weekly,
                declined,
                abroad,
                timed("planning", "Sprint planning", day(0), (10, 0), 90),
                timed("review", "Design review", day(0), (10, 30), 60),
                timed("pair", "Pairing", day(0), (11, 0), 120),
                timed("one", "1:1", day(1), (14, 0), 30),
                timed("demo", "Demo", day(3), (16, 0), 45),
                timed("retro", "Retro", day(4), (15, 30), 60),
                timed("deploy", "Deploy window", day(1), (22, 0), 240),
                all_day("offsite", "Team offsite", day(2), 2),
                all_day("release", "Release freeze", day(-3), 6),
            ]),
            None,
        )
        .unwrap();
    store
        .apply_events_page(
            "home",
            "personal",
            &items(vec![
                timed("gym", "Gym", day(0), (7, 0), 60),
                timed("lunch", "Lunch with Sam", day(1), (12, 30), 60),
                timed("dentist", "Dentist", day(2), (8, 30), 45),
                timed("dinner", "Dinner", day(4), (19, 0), 120),
                timed("run", "Long run", day(5), (9, 0), 90),
                timed("quick", "Call the bank", day(3), (9, 0), 10),
                all_day("trip", "Weekend trip", day(5), 2),
                all_day("rent", "Pay rent", day(0), 1),
            ]),
            None,
        )
        .unwrap();
    let busy = today.with_day(15).unwrap_or(today);
    let mut family: Vec<Value> = (0..7)
        .map(|n| timed(&format!("busy{n}"), "Errand", busy, (9 + n, 0), 30))
        .collect();
    family.push(timed("school", "School play", day(3), (18, 0), 90));
    family.push(all_day("visit", "Grandparents visit", day(8), 4));
    store
        .apply_events_page("home", "family", &items(family), None)
        .unwrap();
    store
        .apply_events_page(
            "home",
            "birthdays",
            &items(vec![all_day("bday", "Alex's birthday", day(2), 1)]),
            None,
        )
        .unwrap();
}

fn main() {
    let view = std::env::args()
        .nth(1)
        .and_then(|name| ViewKind::parse(&name))
        .unwrap_or_default();
    let dir = tempfile::tempdir().expect("a temporary directory");
    let db = dir.path().join("takt.sqlite3");
    seed(&db);
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        takt::keys::bind(cx);
        let options = WindowOptions {
            titlebar: Some(TitleBar::title_bar_options()),
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(80.), px(60.)),
                size: size(px(1280.), px(820.)),
            })),
            ..Default::default()
        };
        // Never read: nothing syncs. Kept open so the view's channels stay alive.
        let (commands, command_rx) = async_channel::unbounded();
        let (event_tx, events) = async_channel::unbounded();
        let db = db.clone();
        gpui_kit::open_window(options, cx, move |window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            let settings = ViewSettings {
                default_view: view,
                time_zone: Some(chrono_tz::UTC),
                secondary_time_zone: Some(chrono_tz::America::New_York),
                ..ViewSettings::default()
            };
            let main = cx.new(|cx| MainView::new(db, settings, commands, events, window, cx));
            cx.new(|_| Demo { main })
        })
        .expect("cannot open the demo window");
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
        // Leaked on purpose, with the directory: they must outlive the window.
        std::mem::forget((command_rx, event_tx));
    });
    drop(dir);
}
