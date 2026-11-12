//! The editing features driven through a headless window: keys, typing,
//! clicks and drags in, the write sent to sync out.

use chrono::{NaiveDate, TimeDelta, Utc};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, Point, SharedString, TestAppContext,
    WindowBounds, WindowOptions, point, px, size,
};
use serde_json::{Value, json};
use takt_google::model::{CalendarListEntry, EventFields, Item};
use takt_store::When;
use takt_sync::{Command, InstanceOf, SyncEvent, WriteError, WriteOp, WriteOutcome, WriteRequest};

use crate::agenda::RowKey;
use crate::config::ViewSettings;
use crate::ui::editing::Preview;
use crate::ui::editor::EventEditor;
use crate::ui::main_view::MainView;
use crate::view::ViewKind;

struct Rig {
    _dir: tempfile::TempDir,
    db: std::path::PathBuf,
    window: AnyWindowHandle,
    view: Entity<MainView>,
    commands: async_channel::Receiver<Command>,
    today: NaiveDate,
}

fn item(value: Value) -> Item<EventFields> {
    Item::from_value(value).unwrap()
}

/// "Lunch" at 12:00 UTC today, an hour long.
fn lunch(today: NaiveDate) -> Value {
    json!({
        "id": "lunch", "etag": "\"1\"", "summary": "Lunch",
        "start": {"dateTime": format!("{today}T12:00:00Z")},
        "end": {"dateTime": format!("{today}T13:00:00Z")}
    })
}

fn with_guests(mut event: Value, organizer: bool) -> Value {
    event["organizer"] = json!({"email": "me@example.com", "self": organizer});
    event["attendees"] = json!([
        {"email": "me@example.com", "self": true, "responseStatus": "needsAction"},
        {"email": "sam@example.com", "responseStatus": "accepted"}
    ]);
    event
}

impl Rig {
    /// Opens `view` in UTC over a writable calendar "cal", a read-only
    /// "feed", and `events` in "cal". The time grid is scrolled to 11:00.
    fn open(view: ViewKind, events: &[Value], cx: &mut TestAppContext) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("takt.sqlite3");
        let today = Utc::now().date_naive();
        {
            let mut store = takt_store::Store::open(&db).unwrap();
            store.upsert_account("acc", "me@example.com").unwrap();
            let calendars: Vec<Item<CalendarListEntry>> = [
                json!({"id": "cal", "summary": "Me", "selected": true, "primary": true,
                       "accessRole": "owner", "backgroundColor": "#039be5"}),
                json!({"id": "feed", "summary": "Feed", "selected": true, "accessRole": "reader"}),
            ]
            .into_iter()
            .map(|c| Item::from_value(c).unwrap())
            .collect();
            store
                .apply_calendar_list_page("acc", &calendars, None)
                .unwrap();
            let items: Vec<_> = events.iter().cloned().map(item).collect();
            store.apply_events_page("acc", "cal", &items, None).unwrap();
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::keys::bind(cx);
        });
        let (commands, commands_rx) = async_channel::unbounded();
        let (_events_tx, events_rx) = async_channel::unbounded();
        let path = db.clone();
        let (window, main) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    // Tall enough that the whole editor is on screen: the
                    // test helpers only click what is visible.
                    size: size(px(1200.), px(1500.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let settings = ViewSettings {
                    default_view: view,
                    time_zone: Some(chrono_tz::UTC),
                    ..ViewSettings::default()
                };
                cx.new(|cx| MainView::new(path, settings, commands, events_rx, window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        let rig = Rig {
            _dir: dir,
            db,
            window,
            view: main,
            commands: commands_rx,
            today,
        };
        rig.frame(cx);
        let main = rig.view.clone();
        cx.update_window(rig.window, |_, window, cx| {
            let scroll = main.read(cx).grid_scroll.clone();
            scroll.set_offset(point(px(0.), px(-11. * crate::ui::time_grid::HOUR_HEIGHT)));
            window.refresh();
            window.render_frame(cx);
        })
        .unwrap();
        while rig.commands.try_recv().is_ok() {}
        rig
    }

    fn frame(&self, cx: &mut TestAppContext) {
        cx.update_window(self.window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
        cx.update_window(self.window, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn act(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
    ) {
        cx.update_window(self.window, |_, window, cx| f(window, cx))
            .unwrap();
        cx.run_until_parked();
        self.frame(cx);
    }

    fn click(&self, id: impl Into<String>, cx: &mut TestAppContext) {
        let id = SharedString::from(id.into());
        self.act(cx, |window, cx| window.click(id, cx));
    }

    fn double_click(&self, id: impl Into<String>, cx: &mut TestAppContext) {
        let id = SharedString::from(id.into());
        self.act(cx, |window, cx| window.double_click(id, cx));
    }

    fn drag(&self, from: impl Into<String>, to: impl Into<String>, cx: &mut TestAppContext) {
        let (from, to) = (
            SharedString::from(from.into()),
            SharedString::from(to.into()),
        );
        self.act(cx, |window, cx| window.drag_to(from, to, cx));
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.press(key, cx));
    }

    fn input(&self, text: &str, cx: &mut TestAppContext) {
        self.act(cx, |window, cx| window.input(text, cx));
    }

    fn block(&self, id: &str) -> String {
        format!("block-acc-cal-{id}-{}", self.today)
    }

    fn slot(&self, hour: u32) -> String {
        format!("slot-{}-{hour:02}", self.today)
    }

    fn editor(&self, cx: &mut TestAppContext) -> Option<Entity<EventEditor>> {
        cx.read(|cx| self.view.read(cx).editor.clone())
    }

    /// Replaces an editor input's text, as selecting all and typing would.
    fn set(
        &self,
        cx: &mut TestAppContext,
        field: impl FnOnce(&EventEditor) -> Entity<gpui_kit::component::input::InputState>,
        value: &str,
    ) {
        let editor = self.editor(cx).expect("the editor is open");
        self.act(cx, |window, cx| {
            let input = field(editor.read(cx));
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        });
    }

    fn shown_view(&self, cx: &mut TestAppContext) -> ViewKind {
        cx.read(|cx| self.view.read(cx).model().read(cx).view())
    }

    fn selected(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.read(|cx| match self.view.read(cx).selected_event() {
            Some(RowKey::Event { event_id, .. }) => Some(event_id.clone()),
            _ => None,
        })
    }

    fn notice(&self, cx: &mut TestAppContext) -> Option<String> {
        cx.read(|cx| self.view.read(cx).last_notice.clone())
    }

    /// The one write sent since the last call.
    fn write(&self) -> WriteRequest {
        match self.commands.try_recv() {
            Ok(Command::Write(request)) => *request,
            other => panic!("expected a write, got {other:?}"),
        }
    }

    fn no_write(&self) {
        assert!(self.commands.try_recv().is_err(), "nothing is sent");
    }

    /// What sync does after Google accepts a write: store the event, say
    /// what changed, then answer.
    fn accept(&self, id: u64, event: Value, cx: &mut TestAppContext) {
        let event_id = event["id"].as_str().unwrap().to_owned();
        let changes = takt_store::Store::open(&self.db)
            .unwrap()
            .apply_event("acc", "cal", &item(event))
            .unwrap();
        self.answer(
            id,
            Some(changes),
            Ok(WriteOutcome {
                calendar_id: "cal".into(),
                event_id,
            }),
            cx,
        );
    }

    fn answer(
        &self,
        id: u64,
        changes: Option<takt_store::ChangeSet>,
        result: Result<WriteOutcome, WriteError>,
        cx: &mut TestAppContext,
    ) {
        let model = cx.read(|cx| self.view.read(cx).model().clone());
        model.update(cx, |model, cx| {
            if let Some(changes) = changes {
                model.handle_sync_event(
                    SyncEvent::Changed {
                        account_id: "acc".into(),
                        changes,
                    },
                    cx,
                );
            }
            model.handle_sync_event(SyncEvent::WriteDone { id, result }, cx);
        });
        cx.run_until_parked();
        self.frame(cx);
    }
}

fn patch_of(request: &WriteRequest) -> (&str, Option<&str>, &Value) {
    match &request.op {
        WriteOp::Update {
            event_id,
            etag,
            patch,
            ..
        } => (event_id, etag.as_deref(), patch),
        other => panic!("expected an update, got {other:?}"),
    }
}

// ---- The editor ----

#[gpui_kit::test]
fn c_opens_a_new_event_and_typing_does_not_trigger_view_keys(cx: &mut TestAppContext) {
    let rig = Rig::open(ViewKind::Week, &[], cx);
    rig.press("c", cx);
    let editor = rig.editor(cx).expect("c opens the editor");
    assert!(cx.read(|cx| editor.read(cx).is_new()));

    // d, w, m, a, t, c and e are all keys of the main view.
    rig.input("Team decamp, wet", cx);
    assert_eq!(rig.shown_view(cx), ViewKind::Week);
    assert_eq!(
        cx.read(|cx| editor.read(cx).title.read(cx).value().to_string()),
        "Team decamp, wet"
    );

    rig.press("enter", cx);
    let request = rig.write();
    assert_eq!(
        (request.account_id.as_str(), request.calendar_id.as_str()),
        ("acc", "cal")
    );
    assert!(!request.notify_guests);
    let WriteOp::Create { body } = &request.op else {
        panic!("a new event is created")
    };
    assert_eq!(body["summary"], "Team decamp, wet");
    assert!(body["start"]["dateTime"].is_string());
    assert_eq!(body["start"]["timeZone"], "UTC");
    assert!(
        cx.read(|cx| editor.read(cx).saving),
        "it waits for the answer"
    );

    let mut created = lunch(rig.today);
    created["id"] = "new01".into();
    rig.accept(request.id, created, cx);
    assert!(rig.editor(cx).is_none(), "a saved editor closes");
    assert_eq!(rig.selected(cx).as_deref(), Some("new01"));
    // The grid has the keys again.
    rig.press("m", cx);
    assert_eq!(rig.shown_view(cx), ViewKind::Month);
}

#[gpui_kit::test]
fn e_edits_the_selected_event_and_sends_only_what_changed(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.press("tab", cx);
    assert_eq!(rig.selected(cx).as_deref(), Some("lunch"));
    rig.press("e", cx);
    let editor = rig.editor(cx).expect("e opens the editor");
    cx.read(|cx| {
        let editor = editor.read(cx);
        assert!(!editor.is_new());
        assert_eq!(editor.title.read(cx).value().as_ref(), "Lunch");
        assert_eq!(editor.start_time.read(cx).value().as_ref(), "12:00");
        assert!(!editor.is_dirty(cx));
    });

    rig.set(cx, |e| e.title.clone(), "Late lunch");
    rig.set(cx, |e| e.location.clone(), "Cafe");
    rig.click("editor-save", cx);
    let request = rig.write();
    let (event_id, etag, patch) = patch_of(&request);
    assert_eq!((event_id, etag), ("lunch", Some("\"1\"")));
    assert_eq!(patch, &json!({"summary": "Late lunch", "location": "Cafe"}));
    let WriteOp::Update { base, move_to, .. } = &request.op else {
        unreachable!()
    };
    assert_eq!(base, &json!({"summary": "Lunch", "location": null}));
    assert_eq!(move_to, &None);
}

#[gpui_kit::test]
fn saving_an_untouched_event_sends_nothing(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.double_click(rig.block("lunch"), cx);
    assert!(rig.editor(cx).is_some(), "a double click opens the editor");
    rig.press("secondary-enter", cx);
    rig.no_write();
    assert!(rig.editor(cx).is_none());
}

#[gpui_kit::test]
fn moving_the_start_moves_the_end_and_times_are_sent_in_utc(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.double_click(rig.block("lunch"), cx);
    let editor = rig.editor(cx).unwrap();
    rig.set(cx, |e| e.start_time.clone(), "2pm");
    rig.press("secondary-enter", cx);
    let request = rig.write();
    let (_, _, patch) = patch_of(&request);
    assert_eq!(
        patch["start"]["dateTime"],
        format!("{today}T14:00:00Z").as_str()
    );
    assert_eq!(
        patch["end"]["dateTime"],
        format!("{today}T15:00:00Z").as_str(),
        "the event keeps its length"
    );
    assert_eq!(
        cx.read(|cx| editor.read(cx).end_time.read(cx).value().to_string()),
        "15:00"
    );
}

#[gpui_kit::test]
fn a_time_takt_cannot_read_is_named_and_nothing_is_sent(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.double_click(rig.block("lunch"), cx);
    let editor = rig.editor(cx).unwrap();
    rig.set(cx, |e| e.end_time.clone(), "late");
    rig.click("editor-save", cx);
    rig.no_write();
    let problem = cx.read(|cx| editor.read(cx).problem.clone()).unwrap();
    assert!(problem.contains("\"late\""), "{problem}");

    rig.set(cx, |e| e.end_time.clone(), "11:00");
    rig.click("editor-save", cx);
    rig.no_write();
    assert_eq!(
        cx.read(|cx| editor.read(cx).problem.clone()).as_deref(),
        Some("The event ends before it starts.")
    );
}

#[gpui_kit::test]
fn escape_closes_a_clean_editor_and_asks_before_dropping_changes(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.double_click(rig.block("lunch"), cx);
    rig.press("escape", cx);
    assert!(rig.editor(cx).is_none());

    rig.double_click(rig.block("lunch"), cx);
    rig.set(cx, |e| e.title.clone(), "Changed");
    rig.click("editor-cancel", cx);
    assert!(rig.editor(cx).is_some(), "it asks first");
    rig.click("ok", cx);
    assert!(rig.editor(cx).is_none());
    rig.no_write();
}

#[gpui_kit::test]
fn a_second_edit_waits_for_the_first(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.double_click(rig.block("lunch"), cx);
    let first = rig.editor(cx).unwrap();
    rig.set(cx, |e| e.title.clone(), "Changed");
    // Back in the grid, with the editor still open.
    rig.click(rig.block("lunch"), cx);
    rig.press("c", cx);
    assert_eq!(rig.editor(cx), Some(first), "the typed changes are kept");
    assert!(rig.notice(cx).unwrap().contains("Save or cancel"));
}

#[gpui_kit::test]
fn guests_reply_meet_and_reminders_are_sent_as_typed(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.double_click(rig.block("lunch"), cx);
    let editor = rig.editor(cx).unwrap();

    rig.act(cx, |window, cx| {
        let guest = editor.read(cx).guest.clone();
        guest.update(cx, |guest, cx| guest.focus(window, cx));
    });
    rig.input("kim@example.com", cx);
    rig.press("enter", cx);
    cx.read(|cx| {
        let guests: Vec<_> = editor
            .read(cx)
            .guests
            .iter()
            .map(|g| g.email.clone())
            .collect();
        // The primary calendar's owner joins as the first guest, like on the web.
        assert_eq!(guests, ["me@example.com", "kim@example.com"]);
    });
    rig.click("editor-reply-tentative", cx);
    rig.click("editor-meet", cx);
    rig.click("editor-add-reminder", cx);
    rig.click("editor-reminder-unit-0", cx);
    rig.click("editor-save", cx);

    let request = rig.write();
    assert!(
        request.notify_guests,
        "new guests are emailed unless unticked"
    );
    let (_, _, patch) = patch_of(&request);
    assert_eq!(
        patch["attendees"],
        json!([
            {"email": "me@example.com", "responseStatus": "tentative"},
            {"email": "kim@example.com"}
        ])
    );
    assert_eq!(
        patch["reminders"],
        json!({"useDefault": false, "overrides": [{"method": "popup", "minutes": 600}]})
    );
    assert_eq!(
        patch["conferenceData"]["createRequest"]["conferenceSolutionKey"]["type"],
        "hangoutsMeet"
    );
    assert_eq!(patch.as_object().unwrap().len(), 3);
}

#[gpui_kit::test]
fn unticking_the_email_box_saves_quietly(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[with_guests(lunch(today), true)], cx);
    rig.double_click(rig.block("lunch"), cx);
    rig.set(cx, |e| e.title.clone(), "Late lunch");
    rig.click("editor-notify", cx);
    rig.click("editor-save", cx);
    assert!(!rig.write().notify_guests);
}

#[gpui_kit::test]
fn a_refused_write_keeps_the_editor_open_with_googles_reason(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.double_click(rig.block("lunch"), cx);
    let editor = rig.editor(cx).unwrap();
    rig.set(cx, |e| e.title.clone(), "Late lunch");
    rig.click("editor-save", cx);
    let request = rig.write();
    rig.answer(request.id, None, Err(WriteError::Offline), cx);
    cx.read(|cx| {
        let editor = editor.read(cx);
        assert!(!editor.saving);
        assert!(editor.problem.as_deref().unwrap().contains("no connection"));
        assert_eq!(editor.title.read(cx).value().as_ref(), "Late lunch");
    });
    // Saving again sends it again.
    rig.click("editor-save", cx);
    assert_eq!(patch_of(&rig.write()).2, &json!({"summary": "Late lunch"}));
}

#[gpui_kit::test]
fn a_conflict_keeps_the_typed_values_and_the_next_save_goes_over_googles(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.double_click(rig.block("lunch"), cx);
    let editor = rig.editor(cx).unwrap();
    rig.set(cx, |e| e.title.clone(), "Late lunch");
    rig.click("editor-save", cx);
    let request = rig.write();

    // Someone renamed it meanwhile; sync stored Google's version and refused.
    let mut theirs = lunch(today);
    theirs["etag"] = "\"2\"".into();
    theirs["summary"] = "Brunch".into();
    let changes = takt_store::Store::open(&rig.db)
        .unwrap()
        .apply_event("acc", "cal", &item(theirs))
        .unwrap();
    rig.answer(request.id, Some(changes), Err(WriteError::Conflict), cx);
    cx.read(|cx| {
        let editor = editor.read(cx);
        assert!(!editor.saving);
        let notice = editor.notice.as_deref().expect("the conflict is explained");
        assert!(notice.contains("title"), "{notice}");
        assert_eq!(editor.title.read(cx).value().as_ref(), "Late lunch");
    });

    rig.click("editor-save", cx);
    let again = rig.write();
    let (_, etag, patch) = patch_of(&again);
    assert_eq!(etag, Some("\"2\""), "against the version now in the cache");
    assert_eq!(patch, &json!({"summary": "Late lunch"}));
    let WriteOp::Update { base, .. } = &again.op else {
        unreachable!()
    };
    assert_eq!(base, &json!({"summary": "Brunch"}));
}

#[gpui_kit::test]
fn an_event_deleted_elsewhere_closes_the_editor(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.double_click(rig.block("lunch"), cx);
    rig.set(cx, |e| e.title.clone(), "Late lunch");
    rig.click("editor-save", cx);
    let request = rig.write();
    rig.answer(request.id, None, Err(WriteError::Gone), cx);
    assert!(rig.editor(cx).is_none());
    assert!(rig.notice(cx).unwrap().contains("deleted in Google"));
}

#[gpui_kit::test]
fn an_occurrence_is_edited_and_deleted_on_its_own(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let mut series = lunch(today);
    series["id"] = "weekly".into();
    series["recurrence"] = json!(["RRULE:FREQ=DAILY"]);
    let rig = Rig::open(ViewKind::Day, &[series], cx);
    let instance = format!("weekly_{}T120000Z", today.format("%Y%m%d"));
    rig.double_click(rig.block(&instance), cx);
    rig.set(cx, |e| e.title.clone(), "Lunch out");
    rig.click("editor-save", cx);
    let request = rig.write();
    let (event_id, etag, patch) = patch_of(&request);
    assert_eq!(event_id, instance, "the write goes to the occurrence");
    assert_eq!(etag, None);
    assert_eq!(patch, &json!({"summary": "Lunch out"}));
    rig.answer(request.id, None, Err(WriteError::Offline), cx);

    rig.click("editor-delete", cx);
    rig.click("ok", cx);
    let request = rig.write();
    let start = today.and_hms_opt(12, 0, 0).unwrap().and_utc();
    assert_eq!(
        request.op,
        WriteOp::Delete {
            event_id: instance,
            instance: Some(InstanceOf {
                series_id: "weekly".into(),
                original_start: When::At(start),
            }),
        }
    );
    rig.answer(
        request.id,
        None,
        Ok(WriteOutcome {
            calendar_id: "cal".into(),
            event_id: "weekly".into(),
        }),
        cx,
    );
    assert!(
        rig.editor(cx).is_none(),
        "deleting from the editor closes it"
    );
}

// ---- Delete and reply without the editor ----

#[gpui_kit::test]
fn delete_asks_first(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.press("tab", cx);
    rig.press("delete", cx);
    rig.no_write();
    rig.click("ok", cx);
    let request = rig.write();
    assert_eq!(
        request.op,
        WriteOp::Delete {
            event_id: "lunch".into(),
            instance: None
        }
    );
    assert!(!request.notify_guests);
}

#[gpui_kit::test]
fn deleting_an_event_with_guests_offers_to_email_them(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[with_guests(lunch(today), true)], cx);
    rig.click(rig.block("lunch"), cx);
    rig.click("detail-delete", cx);
    rig.click("guests-cancel", cx);
    rig.no_write();

    rig.click("detail-delete", cx);
    rig.click("guests-notify", cx);
    assert!(rig.write().notify_guests);

    rig.click("detail-delete", cx);
    rig.click("guests-quiet", cx);
    assert!(!rig.write().notify_guests);
}

#[gpui_kit::test]
fn a_reply_changes_only_the_accounts_own_status(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let invitation = with_guests(lunch(today), false);
    let rig = Rig::open(ViewKind::Week, &[invitation], cx);
    rig.click(rig.block("lunch"), cx);
    rig.click("detail-reply-accepted", cx);
    let request = rig.write();
    assert!(!request.notify_guests);
    let (_, etag, patch) = patch_of(&request);
    assert_eq!(etag, Some("\"1\""));
    assert_eq!(
        patch,
        &json!({"attendees": [
            {"email": "me@example.com", "self": true, "responseStatus": "accepted"},
            {"email": "sam@example.com", "responseStatus": "accepted"}
        ]})
    );
}

#[gpui_kit::test]
fn a_read_only_calendars_events_cannot_be_changed(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[], cx);
    takt_store::Store::open(&rig.db)
        .unwrap()
        .apply_event("acc", "feed", &item(lunch(today)))
        .unwrap();
    let model = cx.read(|cx| rig.view.read(cx).model().clone());
    model.update(cx, |model, cx| model.reload(cx));
    cx.run_until_parked();
    rig.frame(cx);

    let block = format!("block-acc-feed-lunch-{today}");
    rig.click(block.clone(), cx);
    rig.press("e", cx);
    assert!(rig.editor(cx).is_none());
    assert_eq!(
        rig.notice(cx).as_deref(),
        Some("This calendar is read-only.")
    );
    rig.drag(block, rig.slot(15), cx);
    rig.no_write();
}

// ---- Dragging ----

#[gpui_kit::test]
fn dragging_a_block_moves_the_event_and_shows_it_there_until_saved(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    // From the middle of 12:00–13:00 to the middle of the 14:00 hour.
    rig.drag(rig.block("lunch"), rig.slot(14), cx);
    let request = rig.write();
    let (event_id, etag, patch) = patch_of(&request);
    assert_eq!((event_id, etag), ("lunch", Some("\"1\"")));
    assert_eq!(
        patch,
        &json!({
            "start": {"dateTime": format!("{today}T14:00:00Z"), "timeZone": "UTC", "date": null},
            "end": {"dateTime": format!("{today}T15:00:00Z"), "timeZone": "UTC", "date": null},
        })
    );
    assert_eq!(rig.selected(cx), None, "the drag's click selects nothing");
    let at = |hour| today.and_hms_opt(hour, 0, 0).unwrap();
    let preview = cx.read(|cx| rig.view.read(cx).drag.as_ref().map(|d| d.preview));
    assert_eq!(
        preview,
        Some(Preview::Timed {
            start: at(14),
            end: at(15)
        }),
        "it stays where it was dropped while the write runs"
    );

    let mut moved = lunch(today);
    moved["etag"] = "\"2\"".into();
    moved["start"]["dateTime"] = format!("{today}T14:00:00Z").into();
    moved["end"]["dateTime"] = format!("{today}T15:00:00Z").into();
    rig.accept(request.id, moved, cx);
    assert!(cx.read(|cx| rig.view.read(cx).drag.is_none()));
}

#[gpui_kit::test]
fn a_failed_move_puts_the_event_back_and_says_why(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.drag(rig.block("lunch"), rig.slot(14), cx);
    let request = rig.write();
    rig.answer(request.id, None, Err(WriteError::Conflict), cx);
    assert!(cx.read(|cx| rig.view.read(cx).drag.is_none()));
    assert!(
        rig.notice(cx)
            .unwrap()
            .starts_with("The event was not moved")
    );
}

#[gpui_kit::test]
fn dragging_a_blocks_bottom_edge_changes_only_its_end(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[lunch(today)], cx);
    rig.drag(format!("resize-acc-cal-lunch-{today}"), rig.slot(15), cx);
    let request = rig.write();
    let (_, _, patch) = patch_of(&request);
    assert_eq!(
        patch,
        &json!({"end": {
            "dateTime": format!("{today}T15:30:00Z"), "timeZone": "UTC", "date": null
        }})
    );
}

#[gpui_kit::test]
fn moving_an_event_with_guests_asks_about_the_email(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let rig = Rig::open(ViewKind::Week, &[with_guests(lunch(today), true)], cx);
    rig.drag(rig.block("lunch"), rig.slot(14), cx);
    rig.no_write();
    rig.click("guests-cancel", cx);
    rig.no_write();
    assert!(
        cx.read(|cx| rig.view.read(cx).drag.is_none()),
        "it goes back"
    );

    rig.drag(rig.block("lunch"), rig.slot(14), cx);
    rig.click("guests-notify", cx);
    assert!(rig.write().notify_guests);
}

#[gpui_kit::test]
fn dragging_on_empty_grid_draws_a_new_event(cx: &mut TestAppContext) {
    let rig = Rig::open(ViewKind::Week, &[], cx);
    rig.drag(rig.slot(14), rig.slot(16), cx);
    let editor = rig.editor(cx).expect("the editor opens on the drawn time");
    cx.read(|cx| {
        let editor = editor.read(cx);
        assert!(editor.is_new());
        assert_eq!(editor.start_time.read(cx).value().as_ref(), "14:30");
        assert_eq!(editor.end_time.read(cx).value().as_ref(), "16:30");
    });
    rig.input("Workshop", cx);
    rig.press("enter", cx);
    let WriteOp::Create { body } = rig.write().op else {
        panic!("a new event is created")
    };
    assert_eq!(
        body["start"]["dateTime"],
        format!("{}T14:30:00Z", rig.today).as_str()
    );
}

#[gpui_kit::test]
fn a_double_click_on_empty_grid_starts_an_hour_long_event(cx: &mut TestAppContext) {
    let rig = Rig::open(ViewKind::Week, &[], cx);
    rig.double_click(rig.slot(15), cx);
    let editor = rig.editor(cx).expect("the editor opens");
    cx.read(|cx| {
        let editor = editor.read(cx);
        assert_eq!(editor.start_time.read(cx).value().as_ref(), "15:30");
        assert_eq!(editor.end_time.read(cx).value().as_ref(), "16:30");
    });
}

#[gpui_kit::test]
fn in_the_month_a_bar_is_dragged_to_another_day(cx: &mut TestAppContext) {
    let today = Utc::now().date_naive();
    let holiday = json!({
        "id": "holiday", "etag": "\"1\"", "summary": "Holiday",
        "start": {"date": today.to_string()},
        "end": {"date": (today + TimeDelta::days(1)).to_string()}
    });
    let rig = Rig::open(ViewKind::Month, &[holiday], cx);
    let (from, to) = cx.read(|cx| {
        let model = rig.view.read(cx).model().read(cx);
        (model.from, model.to)
    });
    // Another day of the same week, so the bar's row ID is known.
    let week = from + TimeDelta::days((today - from).num_days() / 7 * 7);
    let target = if today == week {
        today + TimeDelta::days(1)
    } else {
        today - TimeDelta::days(1)
    };
    assert!(target >= from && target < to);
    rig.drag(
        format!("bar-acc-cal-holiday-{week}"),
        format!("num-{target}"),
        cx,
    );
    let request = rig.write();
    let (_, _, patch) = patch_of(&request);
    assert_eq!(
        patch,
        &json!({
            "start": {"date": target.to_string(), "dateTime": null, "timeZone": null},
            "end": {"date": (target + TimeDelta::days(1)).to_string(), "dateTime": null, "timeZone": null},
        })
    );
}

#[gpui_kit::test]
fn in_the_month_c_and_a_double_click_start_all_day_events(cx: &mut TestAppContext) {
    let rig = Rig::open(ViewKind::Month, &[], cx);
    rig.press("c", cx);
    let editor = rig.editor(cx).unwrap();
    assert!(cx.read(|cx| editor.read(cx).all_day));
    rig.press("escape", cx);
    assert!(rig.editor(cx).is_none());

    let first = cx.read(|cx| rig.view.read(cx).model().read(cx).from);
    rig.double_click(format!("day-{first}"), cx);
    let editor = rig
        .editor(cx)
        .expect("a double click on a day opens the editor");
    cx.read(|cx| {
        let editor = editor.read(cx);
        assert!(editor.all_day);
        assert_eq!(
            editor.start_date.read(cx).value().as_ref(),
            crate::entry::format_date(first)
        );
    });
}
