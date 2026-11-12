//! Opens takt's main view over made-up events, with no Google account and no
//! sync. For looking at the views and the editor while working on them:
//!
//! ```text
//! cargo run -p takt --example demo -- month
//! ```
//!
//! The argument is the view to open in: day, week (the default), month or agenda.
//! Add `new` to open with the editor on a new event.
//!
//! Changes are answered by a stand-in for the sync thread that applies them
//! to the temporary cache, so creating, editing, dragging and deleting can be
//! tried. It cannot change one occurrence of a repeating event.

use chrono::{Datelike as _, NaiveDate, TimeDelta, Utc};
use gpui_kit::component::{ActiveTheme as _, Theme, TitleBar, v_flex};
use gpui_kit::*;
use serde_json::{Value, json};
use takt::config::ViewSettings;
use takt::ui::MainView;
use takt::view::ViewKind;
use takt_google::model::{CalendarListEntry, Colors, EventFields, Item, Setting};
use takt_store::{ChangeSet, Store};
use takt_sync::{Command, SyncEvent, WriteError, WriteOp, WriteOutcome, WriteRequest};

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
    // Google's event colours, so the editor has swatches.
    let palette: Item<Colors> = Item::from_value(json!({"calendar": {}, "event": {
        "1": {"background": "#a4bdfc", "foreground": "#1d1d1d"},
        "2": {"background": "#7ae7bf", "foreground": "#1d1d1d"},
        "3": {"background": "#dbadff", "foreground": "#1d1d1d"},
        "4": {"background": "#ff887c", "foreground": "#1d1d1d"},
        "5": {"background": "#fbd75b", "foreground": "#1d1d1d"},
        "6": {"background": "#ffb878", "foreground": "#1d1d1d"},
        "7": {"background": "#46d6db", "foreground": "#1d1d1d"},
        "8": {"background": "#e1e1e1", "foreground": "#1d1d1d"},
        "9": {"background": "#5484ed", "foreground": "#1d1d1d"},
        "10": {"background": "#51b749", "foreground": "#1d1d1d"},
        "11": {"background": "#dc2127", "foreground": "#1d1d1d"}
    }}))
    .unwrap();
    store.put_colors("home", &palette).unwrap();
    store.put_colors("work", &palette).unwrap();
    store
        .apply_calendar_list_page(
            "home",
            &[
                calendar(
                    json!({"id": "personal", "summary": "Personal", "primary": true,
                    "accessRole": "owner", "selected": true, "backgroundColor": "#7986cb"}),
                ),
                calendar(
                    json!({"id": "family", "summary": "Family", "selected": true,
                    "accessRole": "writer", "backgroundColor": "#33b679"}),
                ),
                calendar(
                    json!({"id": "birthdays", "summary": "Birthdays", "selected": false,
                    "accessRole": "reader", "backgroundColor": "#f6bf26"}),
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
                "accessRole": "owner", "selected": true, "backgroundColor": "#e67c73"}),
            )],
            None,
        )
        .unwrap();

    let mut weekly = timed("standup", "Standup", day(-28), (9, 0), 15);
    weekly["recurrence"] = json!(["RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"]);
    let mut declined = timed("optional", "Optional sync", day(3), (15, 0), 60);
    declined["attendees"] = json!([{"email": "you@work.example", "self": true,
        "responseStatus": "declined"}]);
    // An event this account organizes, with a guest: changes offer to email.
    let mut one = timed("one", "1:1", day(1), (14, 0), 30);
    one["organizer"] = json!({"email": "you@work.example", "self": true});
    one["attendees"] = json!([
        {"email": "you@work.example", "self": true, "organizer": true, "responseStatus": "accepted"},
        {"email": "sam@work.example", "displayName": "Sam", "responseStatus": "needsAction"}
    ]);
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
                one,
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

/// JSON merge patch: `null` removes a member, objects merge, the rest replace.
fn merge(target: &mut Value, patch: &Value) {
    let (Some(target_map), Some(patch_map)) = (target.as_object_mut(), patch.as_object()) else {
        *target = patch.clone();
        return;
    };
    for (key, value) in patch_map {
        if value.is_null() {
            target_map.remove(key);
        } else if value.is_object() && target_map.get(key).is_some_and(Value::is_object) {
            merge(&mut target_map[key], value);
        } else {
            target_map.insert(key.clone(), value.clone());
        }
    }
}

/// What Google would do with one write, done to the cache instead.
fn apply(
    store: &mut Store,
    request: &WriteRequest,
    version: u32,
) -> Result<(ChangeSet, WriteOutcome), String> {
    let (account, calendar) = (request.account_id.as_str(), request.calendar_id.as_str());
    let put = |store: &mut Store, calendar: &str, mut raw: Value| -> Result<ChangeSet, String> {
        raw["etag"] = format!("\"demo-{version}\"").into();
        let item = Item::from_value(raw).map_err(|e| e.to_string())?;
        store
            .apply_event(account, calendar, &item)
            .map_err(|e| e.to_string())
    };
    let done = |calendar: &str, event_id: &str| WriteOutcome {
        calendar_id: calendar.to_owned(),
        event_id: event_id.to_owned(),
    };
    match &request.op {
        WriteOp::Create { body } => {
            let mut raw = body.clone();
            let id = takt_google::new_event_id();
            raw["id"] = id.clone().into();
            if raw["conferenceData"]["createRequest"].is_object() {
                raw["hangoutLink"] = "https://meet.google.com/demo-link".into();
            }
            Ok((put(store, calendar, raw)?, done(calendar, &id)))
        }
        WriteOp::Update {
            event_id,
            patch,
            move_to,
            ..
        } => {
            let mut raw = store
                .event_raw(account, calendar, event_id)
                .map_err(|e| e.to_string())?
                .ok_or("the demo cannot change one occurrence of a repeating event")?;
            merge(&mut raw, patch);
            if raw["conferenceData"]["createRequest"].is_object() {
                raw["hangoutLink"] = "https://meet.google.com/demo-link".into();
            }
            let mut changes = put(store, calendar, raw.clone())?;
            let Some(destination) = move_to else {
                return Ok((changes, done(calendar, event_id)));
            };
            let gone = store
                .mark_event_deleted(account, calendar, event_id, None)
                .map_err(|e| e.to_string())?;
            changes.merge(gone);
            changes.merge(put(store, destination, raw)?);
            Ok((changes, done(destination, event_id)))
        }
        WriteOp::Delete { event_id, instance } => {
            let instance = instance
                .as_ref()
                .map(|i| (i.series_id.as_str(), i.original_start));
            let changes = store
                .mark_event_deleted(account, calendar, event_id, instance)
                .map_err(|e| e.to_string())?;
            Ok((changes, done(calendar, event_id)))
        }
    }
}

/// Stands in for the sync thread: answers each change from the window.
fn answer_commands(
    db: std::path::PathBuf,
    commands: async_channel::Receiver<Command>,
    events: async_channel::Sender<SyncEvent>,
) {
    std::thread::spawn(move || {
        let mut store = Store::open(&db).expect("the demo cache opens");
        let mut version = 0;
        let changed = |account_id: &str, changes: ChangeSet| {
            let _ = events.send_blocking(SyncEvent::Changed {
                account_id: account_id.to_owned(),
                changes,
            });
        };
        while let Ok(command) = commands.recv_blocking() {
            match command {
                Command::SetCalendarVisible {
                    account_id,
                    calendar_id,
                    visible,
                } => {
                    if let Ok(changes) =
                        store.set_calendar_visible(&account_id, &calendar_id, Some(visible))
                    {
                        changed(&account_id, changes);
                    }
                }
                Command::Write(request) => {
                    version += 1;
                    let result = match apply(&mut store, &request, version) {
                        Ok((changes, outcome)) => {
                            changed(&request.account_id, changes);
                            Ok(outcome)
                        }
                        Err(message) => Err(WriteError::Rejected(message)),
                    };
                    let _ = events.send_blocking(SyncEvent::WriteDone {
                        id: request.id,
                        result,
                    });
                }
                _ => {}
            }
        }
    });
}

fn main() {
    let view = std::env::args()
        .nth(1)
        .and_then(|name| ViewKind::parse(&name))
        .unwrap_or_default();
    let open_editor = std::env::args().any(|arg| arg == "new");
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
        let (commands, command_rx) = async_channel::unbounded();
        let (event_tx, events) = async_channel::unbounded();
        let db = db.clone();
        answer_commands(db.clone(), command_rx, event_tx);
        gpui_kit::open_window(options, cx, move |window, cx| {
            Theme::sync_system_appearance(Some(window), cx);
            let settings = ViewSettings {
                default_view: view,
                time_zone: Some(chrono_tz::UTC),
                secondary_time_zone: Some(chrono_tz::America::New_York),
                ..ViewSettings::default()
            };
            let main = cx.new(|cx| MainView::new(db, settings, commands, events, window, cx));
            if open_editor {
                // Once the first read has landed, so there are calendars to offer.
                let main = main.clone();
                window
                    .spawn(cx, async move |cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(500))
                            .await;
                        let _ = cx.update(|window, cx| {
                            main.update(cx, |view, cx| view.new_event(None, window, cx))
                        });
                    })
                    .detach();
            }
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
    });
    drop(dir);
}
