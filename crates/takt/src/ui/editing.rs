//! Changing events from the main view: opening the editor, sending writes
//! and following their answers, deleting, replying, and dragging.

use std::cell::Cell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Timelike as _, Utc};
use chrono_tz::Tz;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable as _, WindowExt as _, h_flex};
use gpui_kit::*;
use serde_json::Value;
use takt_store::{CalendarRow, ColorPalette, Store, StoreError, When};
use takt_sync::{Command, InstanceOf, WriteError, WriteOp, WriteOutcome, WriteRequest};

use crate::agenda::{DEFAULT_COLOR, RowKey, parse_hex};
use crate::draft::{
    DraftTime, EventDraft, base_of, has_guests_to_notify, local_instant, occurrence_raw,
};
use crate::entry::next_half_hour;
use crate::grid::Chip;
use crate::model::Snapshot;
use crate::ui::editor::{CalendarChoice, EditorEvent, EditorSetup, EventEditor, Existing};
use crate::ui::main_view::{MainView, find_chip};
use crate::ui::time_grid::HOUR_HEIGHT;
use crate::view::ViewKind;

/// Events snap to this many minutes while dragged.
const SNAP_MINUTES: i64 = 15;

/// An event on screen: enough to find its JSON and to say which occurrence
/// of a series it is.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRef {
    pub account_id: String,
    pub calendar_id: String,
    /// The row whose JSON describes it: the series for a generated occurrence.
    pub event_id: String,
    pub instance_id: String,
    pub start: When,
    pub end: When,
    pub title: String,
    pub color: u32,
}

impl EventRef {
    pub fn of_chip(chip: &Chip) -> Option<EventRef> {
        EventRef::new(&chip.key, chip.start, chip.end, &chip.title, chip.color)
    }

    fn new(key: &RowKey, start: When, end: When, title: &str, color: u32) -> Option<EventRef> {
        let RowKey::Event {
            account_id,
            calendar_id,
            event_id,
            instance_id,
        } = key
        else {
            return None;
        };
        Some(EventRef {
            account_id: account_id.clone(),
            calendar_id: calendar_id.clone(),
            event_id: event_id.clone(),
            instance_id: instance_id.clone(),
            start,
            end,
            title: title.to_owned(),
            color,
        })
    }

    pub fn key(&self) -> RowKey {
        RowKey::Event {
            account_id: self.account_id.clone(),
            calendar_id: self.calendar_id.clone(),
            event_id: self.event_id.clone(),
            instance_id: self.instance_id.clone(),
        }
    }
}

/// What a write was for, kept until sync answers it.
pub(super) enum Pending {
    Editor {
        account_id: String,
        calendar_id: String,
        /// `None` for a new event.
        event_id: Option<String>,
    },
    Quick {
        /// What to say when it fails, e.g. "The event was not moved".
        failed: &'static str,
        closes_editor: bool,
    },
}

/// Where a dragged event should go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Moved {
    To(DateTime<Utc>, DateTime<Utc>),
    Days(i64),
}

impl Moved {
    fn apply(self, time: DraftTime, display: Tz) -> DraftTime {
        let days = |n| TimeDelta::days(n);
        match (self, time) {
            (Moved::To(start, end), time) => time.at(start, end, display),
            (Moved::Days(n), DraftTime::AllDay { first, last }) => DraftTime::AllDay {
                first: first + days(n),
                last: last + days(n),
            },
            (Moved::Days(n), DraftTime::Timed { start, end, zone }) => DraftTime::Timed {
                start: start + days(n),
                end: end + days(n),
                zone,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragMode {
    /// A timed event to another time or day of the time grid.
    Move,
    /// A timed event's end.
    Resize,
    /// A new event drawn on an empty part of the time grid.
    Create,
    /// An event to another day, in the month or the all-day strip.
    Days,
}

/// Where a drag would put the event, in the display zone's wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    Timed {
        start: NaiveDateTime,
        end: NaiveDateTime,
    },
    Day(NaiveDate),
}

pub struct Drag {
    /// `None` while a new event is drawn.
    pub event: Option<EventRef>,
    pub mode: DragMode,
    /// The time, or day, under the pointer when the button went down.
    anchor: NaiveDateTime,
    initial: Preview,
    pub preview: Preview,
    pub moved: bool,
    /// Dropped: the preview stays on screen until the write is answered and
    /// the view has been read again, so the event does not jump back first.
    pub settling: bool,
    answered: bool,
}

fn snap(time: NaiveDateTime) -> NaiveDateTime {
    let minutes = i64::from(time.time().num_seconds_from_midnight()) / 60;
    let snapped = (minutes + SNAP_MINUTES / 2) / SNAP_MINUTES * SNAP_MINUTES;
    time.date().and_time(NaiveTime::MIN) + TimeDelta::minutes(snapped)
}

fn is_writable(calendar: &CalendarRow) -> bool {
    !calendar.hidden && matches!(calendar.access_role.as_deref(), Some("owner" | "writer"))
}

/// The calendars events can be saved to, of one account or of all.
pub(super) fn calendar_choices(snapshot: &Snapshot, account: Option<&str>) -> Vec<CalendarChoice> {
    let several = account.is_none() && snapshot.accounts.len() > 1;
    let email = |account_id: &str| {
        snapshot
            .accounts
            .iter()
            .find(|a| a.account_id == account_id)
            .map(|a| a.email.clone())
            .unwrap_or_default()
    };
    let mut choices: Vec<CalendarChoice> = snapshot
        .calendars
        .iter()
        .filter(|c| is_writable(c) && account.is_none_or(|id| c.account_id == id))
        .map(|c| {
            let account_email = email(&c.account_id);
            CalendarChoice {
                label: if several {
                    format!("{} · {account_email}", c.summary)
                } else {
                    c.summary.clone()
                },
                account_id: c.account_id.clone(),
                calendar_id: c.calendar_id.clone(),
                account_email,
                color: c
                    .background_color
                    .as_deref()
                    .and_then(parse_hex)
                    .unwrap_or(DEFAULT_COLOR),
                primary: c.primary,
            }
        })
        .collect();
    // The picker tells choices apart by label.
    for ix in 0..choices.len() {
        if choices[..ix].iter().any(|c| c.label == choices[ix].label) {
            choices[ix].label = format!("{} ({})", choices[ix].label, choices[ix].calendar_id);
        }
    }
    choices
}

/// Each account's event colours in Google's order.
fn event_colors(palettes: &HashMap<String, ColorPalette>) -> HashMap<String, Vec<(String, u32)>> {
    palettes
        .iter()
        .map(|(account, palette)| {
            let mut colors: Vec<(String, u32)> = palette
                .event
                .iter()
                .filter_map(|(id, (background, _))| Some((id.clone(), parse_hex(background)?)))
                .collect();
            colors.sort_by_key(|(id, _)| (id.parse::<u32>().unwrap_or(u32::MAX), id.clone()));
            (account.clone(), colors)
        })
        .collect()
}

/// What a change needs from the cache.
pub(super) struct Read {
    raw: Option<Value>,
    palettes: HashMap<String, ColorPalette>,
}

fn read_for_edit(
    db: &Path,
    event: Option<(String, String, String)>,
    accounts: &[String],
) -> Result<Read, StoreError> {
    let store = Store::open_read_only(db)?;
    let raw = match event {
        Some((account, calendar, id)) => store.event_raw(&account, &calendar, &id)?,
        None => None,
    };
    let mut palettes = HashMap::new();
    for account in accounts {
        palettes.insert(account.clone(), store.palette(account)?);
    }
    Ok(Read { raw, palettes })
}

/// The event as writes see it. An occurrence the series generates has no
/// row of its own: it is read as the series with that occurrence's times and
/// written through its instance ID.
pub(super) fn prepare(raw: Option<Value>, event: &EventRef) -> Result<Existing, String> {
    let raw = raw.ok_or("This event is no longer in takt's cache.")?;
    let repeats = raw["recurrence"]
        .as_array()
        .is_some_and(|rules| !rules.is_empty());
    if repeats && event.instance_id == event.event_id {
        return Err(
            "takt cannot read this event's repeat rule. Change it in Google Calendar.".into(),
        );
    }
    if repeats {
        return Ok(Existing {
            event_id: event.instance_id.clone(),
            etag: None,
            original: occurrence_raw(&raw, &event.instance_id, event.start, event.end),
            instance: Some(InstanceOf {
                series_id: event.event_id.clone(),
                original_start: event.start,
            }),
        });
    }
    Ok(Existing {
        event_id: event.event_id.clone(),
        etag: raw["etag"].as_str().map(str::to_owned),
        original: raw,
        instance: None,
    })
}

/// Called with whether to email the guests.
type Chosen = Rc<dyn Fn(bool, &mut Window, &mut App)>;
type Cancelled = Rc<dyn Fn(&mut Window, &mut App)>;

/// What to ask before a change.
struct Question {
    title: String,
    description: &'static str,
    /// The change in one word, for the buttons: "Delete", "Move".
    verb: &'static str,
    /// The change can email guests: offer to do it with or without.
    guests: bool,
}

/// Asks before a change; `chosen` gets the answer about emailing guests.
fn confirm(
    question: Question,
    chosen: Chosen,
    cancelled: Cancelled,
    window: &mut Window,
    cx: &mut App,
) {
    let Question {
        title,
        description,
        verb,
        guests,
    } = question;
    let answered = Rc::new(Cell::new(false));
    window.open_alert_dialog(cx, move |dialog, _, _| {
        let dialog = dialog.title(title.clone()).description(description);
        // However the dialog goes away without an answer, that is a cancel,
        // told once.
        let cancel = {
            let (answered, cancelled) = (answered.clone(), cancelled.clone());
            move |window: &mut Window, cx: &mut App| {
                if !answered.replace(true) {
                    cancelled(window, cx);
                }
            }
        };
        let on_close = {
            let cancel = cancel.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| cancel(window, cx)
        };
        if !guests {
            let (answered, chosen) = (answered.clone(), chosen.clone());
            return dialog
                .confirm()
                .ok_text(verb)
                .on_ok(move |_, window, cx| {
                    answered.set(true);
                    chosen(false, window, cx);
                    true
                })
                .on_close(on_close);
        }
        let button = |id: &'static str, label: String, notify: Option<bool>| {
            let (answered, chosen, cancel) = (answered.clone(), chosen.clone(), cancel.clone());
            Button::new(id)
                .small()
                .label(label)
                .on_click(move |_, window, cx| {
                    match notify {
                        Some(notify) => {
                            answered.set(true);
                            chosen(notify, window, cx);
                        }
                        None => cancel(window, cx),
                    }
                    window.close_dialog(cx);
                })
        };
        dialog
            .footer(
                h_flex()
                    .w_full()
                    .gap_2()
                    .justify_end()
                    .child(button("guests-cancel", "Cancel".into(), None).ghost())
                    .child(button(
                        "guests-quiet",
                        format!("{verb} without email"),
                        Some(false),
                    ))
                    .child(
                        button(
                            "guests-notify",
                            format!("{verb} and email guests"),
                            Some(true),
                        )
                        .primary(),
                    ),
            )
            .on_close(on_close)
    });
}

impl MainView {
    fn is_writable(&self, account_id: &str, calendar_id: &str, cx: &App) -> bool {
        self.model
            .read(cx)
            .snapshot
            .calendars
            .iter()
            .any(|c| c.account_id == account_id && c.calendar_id == calendar_id && is_writable(c))
    }

    /// Tells the user something went wrong.
    pub(super) fn notice(
        &mut self,
        message: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = message.into();
        window.push_notification(Notification::error(message.clone()), cx);
        self.last_notice = Some(message);
        cx.notify();
    }

    /// The event `key` names, if it is on screen.
    pub(super) fn find_event(&self, key: &RowKey, cx: &App) -> Option<EventRef> {
        let snapshot = &self.model.read(cx).snapshot;
        if let Some(chip) = find_chip(snapshot, key) {
            return EventRef::of_chip(chip);
        }
        snapshot
            .sections
            .iter()
            .flat_map(|section| &section.rows)
            .find(|row| row.key == *key)
            .and_then(|row| {
                let (start, end) = row.times?;
                EventRef::new(key, start, end, &row.title, row.color)
            })
    }

    /// The event the editing keys act on: the selected one, else the one
    /// whose details are open.
    pub(super) fn target_event(&self, cx: &App) -> Option<EventRef> {
        let selected = if self.model.read(cx).snapshot.view == ViewKind::Agenda {
            let list = self.list.read(cx);
            list.selected_index()
                .and_then(|ix| list.delegate().row(ix))
                .map(|row| row.key.clone())
        } else {
            self.selected.clone()
        };
        selected
            .or_else(|| self.detail_key.clone())
            .and_then(|key| self.find_event(&key, cx))
    }

    /// Reads an event's JSON (when `event` is given) and the colour palettes
    /// in the background, then runs `then` with them.
    fn read_then(
        &mut self,
        event: Option<&EventRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, Read, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let model = self.model.read(cx);
        let db = model.db().to_path_buf();
        let accounts: Vec<String> = model
            .snapshot
            .accounts
            .iter()
            .map(|a| a.account_id.clone())
            .collect();
        let ids = event.map(|e| {
            (
                e.account_id.clone(),
                e.calendar_id.clone(),
                e.event_id.clone(),
            )
        });
        let read = cx.background_spawn(async move { read_for_edit(&db, ids, &accounts) });
        cx.spawn_in(window, async move |this, cx| {
            let result = read.await;
            let _ = this.update_in(cx, |view, window, cx| match result {
                Ok(read) => then(view, read, window, cx),
                Err(error) => {
                    tracing::error!(%error, "cannot read the cache for a change");
                    view.drag = None;
                    view.notice(format!("Cannot read the local cache: {error}"), window, cx);
                }
            });
        })
        .detach();
    }

    /// False, with a notice, while an editor holds typed changes.
    fn editor_is_free(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let busy = self.editor.as_ref().is_some_and(|editor| {
            let editor = editor.read(cx);
            editor.saving || editor.is_dirty(cx)
        });
        if busy {
            self.notice(
                "Save or cancel the event you are editing first.",
                window,
                cx,
            );
        }
        !busy
    }

    /// Opens the editor on a new event. Without `time`: all day on the
    /// selected day in the month, else an hour from the next half hour today
    /// or from 09:00 on another day.
    pub fn new_event(
        &mut self,
        time: Option<DraftTime>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editor_is_free(window, cx) {
            return;
        }
        let model = self.model.read(cx);
        let (tz, today, anchor, view) = (model.tz, model.today(), model.anchor(), model.view());
        let use_24h = model.snapshot.use_24h;
        let calendars = calendar_choices(&model.snapshot, None);
        if calendars.is_empty() {
            return self.notice("There is no calendar you can add events to.", window, cx);
        }
        let calendar = calendars.iter().position(|c| c.primary).unwrap_or(0);
        let time = time.unwrap_or_else(|| {
            let day = if view == ViewKind::Agenda {
                today
            } else {
                anchor
            };
            if view == ViewKind::Month {
                return DraftTime::AllDay {
                    first: day,
                    last: day,
                };
            }
            let from = if day == today {
                next_half_hour(Utc::now().with_timezone(&tz).time())
            } else {
                NaiveTime::from_hms_opt(9, 0, 0).unwrap_or_default()
            };
            let start = day.and_time(from);
            DraftTime::Timed {
                start,
                end: start + TimeDelta::hours(1),
                zone: tz,
            }
        });
        self.read_then(None, window, cx, move |view, read, window, cx| {
            let setup = EditorSetup {
                existing: None,
                draft: EventDraft::new(time),
                calendars,
                calendar,
                colors: event_colors(&read.palettes),
                tz,
                use_24h,
                today,
            };
            view.editing = None;
            view.open_editor(setup, window, cx);
        });
    }

    /// Opens the editor on an event that is on screen.
    pub(crate) fn edit_event(
        &mut self,
        event: EventRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editor_is_free(window, cx) {
            return;
        }
        if !self.is_writable(&event.account_id, &event.calendar_id, cx) {
            return self.notice("This calendar is read-only.", window, cx);
        }
        self.read_then(
            Some(&event.clone()),
            window,
            cx,
            move |view, read, window, cx| {
                let model = view.model.read(cx);
                let (tz, today, use_24h) = (model.tz, model.today(), model.snapshot.use_24h);
                let calendars = calendar_choices(&model.snapshot, Some(&event.account_id));
                let prepared = prepare(read.raw, &event).and_then(|existing| {
                    let draft = EventDraft::from_raw(&existing.original, tz)
                        .ok_or("This event has no time takt can read.")?;
                    let calendar = calendars
                        .iter()
                        .position(|c| c.calendar_id == event.calendar_id)
                        .ok_or("This calendar is read-only.")?;
                    Ok((existing, draft, calendar))
                });
                let (existing, draft, calendar) = match prepared {
                    Ok(prepared) => prepared,
                    Err(message) => return view.notice(message, window, cx),
                };
                let setup = EditorSetup {
                    existing: Some(existing),
                    draft,
                    calendars,
                    calendar,
                    colors: event_colors(&read.palettes),
                    tz,
                    use_24h,
                    today,
                };
                view.editing = Some(event);
                view.open_editor(setup, window, cx);
            },
        );
    }

    fn open_editor(&mut self, setup: EditorSetup, window: &mut Window, cx: &mut Context<Self>) {
        let editor = cx.new(|cx| EventEditor::new(setup, window, cx));
        self.editor_events = Some(cx.subscribe_in(
            &editor,
            window,
            |this, _, event: &EditorEvent, window, cx| this.on_editor_event(event, window, cx),
        ));
        editor.update(cx, |editor, cx| editor.focus_title(window, cx));
        // The editor takes the detail panel's place.
        self.detail_load = None;
        self.detail = None;
        self.detail_key = None;
        self.editor = Some(editor);
        cx.notify();
    }

    pub(super) fn close_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        self.editor_events = None;
        self.editing = None;
        self.focus_view(window, cx);
        cx.notify();
    }

    fn on_editor_event(
        &mut self,
        event: &EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            EditorEvent::Save(request) => {
                let event_id = match &request.op {
                    WriteOp::Update { event_id, .. } => Some(event_id.clone()),
                    _ => None,
                };
                self.send_write(
                    request.account_id.clone(),
                    request.calendar_id.clone(),
                    request.op.clone(),
                    request.notify_guests,
                    Pending::Editor {
                        account_id: request.account_id.clone(),
                        calendar_id: request.calendar_id.clone(),
                        event_id,
                    },
                );
            }
            EditorEvent::Close => self.close_editor(window, cx),
            EditorEvent::Delete => {
                if let Some(event) = self.editing.clone() {
                    self.delete_event(event, window, cx);
                }
            }
        }
    }

    fn send_write(
        &mut self,
        account_id: String,
        calendar_id: String,
        op: WriteOp,
        notify_guests: bool,
        pending: Pending,
    ) {
        self.next_write += 1;
        let id = self.next_write;
        self.pending.insert(id, pending);
        let request = WriteRequest {
            id,
            account_id,
            calendar_id,
            op,
            notify_guests,
        };
        if self
            .commands
            .try_send(Command::Write(Box::new(request)))
            .is_err()
        {
            tracing::error!("sync is not running; the change was not sent");
        }
    }

    pub(super) fn on_write_done(
        &mut self,
        id: u64,
        result: &Result<WriteOutcome, WriteError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pending) = self.pending.remove(&id) else {
            return;
        };
        match pending {
            Pending::Editor {
                account_id,
                calendar_id,
                event_id,
            } => {
                let Some(editor) = self.editor.clone() else {
                    return;
                };
                match (result, event_id) {
                    (Ok(outcome), _) => {
                        self.selected = Some(RowKey::Event {
                            account_id,
                            calendar_id: outcome.calendar_id.clone(),
                            event_id: outcome.event_id.clone(),
                            instance_id: outcome.event_id.clone(),
                        });
                        self.close_editor(window, cx);
                    }
                    (Err(WriteError::Conflict), Some(event_id)) => {
                        self.rebase_editor(editor, account_id, calendar_id, event_id, window, cx);
                    }
                    (Err(WriteError::Gone), _) => {
                        self.close_editor(window, cx);
                        self.notice(
                            "The event was deleted in Google while you were editing it.",
                            window,
                            cx,
                        );
                    }
                    (Err(error), _) => {
                        let message = error.to_string();
                        editor.update(cx, |editor, cx| editor.write_failed(message, cx));
                    }
                }
            }
            Pending::Quick {
                failed,
                closes_editor,
            } => match result {
                Ok(_) => {
                    if closes_editor {
                        self.close_editor(window, cx);
                    }
                    // Keep a dropped event's preview until the read that
                    // shows it in its new place has landed.
                    if self.model.read(cx).is_loading() {
                        if let Some(drag) = &mut self.drag {
                            drag.answered = true;
                        }
                    } else {
                        self.drag = None;
                    }
                    cx.notify();
                }
                Err(error) => {
                    self.drag = None;
                    self.notice(format!("{failed}: {error}."), window, cx);
                }
            },
        }
    }

    /// A read landed: a dropped event whose write was answered is now drawn
    /// where it was dropped.
    pub(super) fn settle_drag(&mut self) {
        if self.drag.as_ref().is_some_and(|drag| drag.answered) {
            self.drag = None;
        }
    }

    /// After a conflict the cache holds Google's version; the editor's next
    /// save is made against it.
    fn rebase_editor(
        &mut self,
        editor: Entity<EventEditor>,
        account_id: String,
        calendar_id: String,
        event_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let db = self.model.read(cx).db().to_path_buf();
        let read = cx.background_spawn(async move {
            Store::open_read_only(&db)?.event_raw(&account_id, &calendar_id, &event_id)
        });
        cx.spawn_in(window, async move |_, cx| {
            let raw = read.await;
            editor.update(cx, |editor, cx| match raw {
                Ok(Some(raw)) => editor.rebase(raw, cx),
                _ => editor.write_failed(WriteError::Conflict.to_string(), cx),
            });
        })
        .detach();
    }

    /// Asks, then deletes an event that is on screen. An occurrence of a
    /// series is deleted on its own.
    pub(crate) fn delete_event(
        &mut self,
        event: EventRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_writable(&event.account_id, &event.calendar_id, cx) {
            return self.notice("This calendar is read-only.", window, cx);
        }
        self.read_then(
            Some(&event.clone()),
            window,
            cx,
            move |view, read, window, cx| {
                let existing = match prepare(read.raw, &event) {
                    Ok(existing) => existing,
                    Err(message) => return view.notice(message, window, cx),
                };
                let occurrence = existing.instance.is_some()
                    || existing.original["recurringEventId"].is_string();
                let description = if occurrence {
                    "Only this occurrence of the repeating event is deleted."
                } else {
                    "The event is deleted from Google Calendar."
                };
                let closes_editor = view.editing.as_ref() == Some(&event);
                let op = WriteOp::Delete {
                    event_id: existing.event_id.clone(),
                    instance: existing.instance.clone(),
                };
                let this = cx.entity().downgrade();
                let (account_id, calendar_id) =
                    (event.account_id.clone(), event.calendar_id.clone());
                let chosen = Rc::new(move |notify: bool, _: &mut Window, cx: &mut App| {
                    let _ = this.update(cx, |view, _| {
                        view.send_write(
                            account_id.clone(),
                            calendar_id.clone(),
                            op.clone(),
                            notify,
                            Pending::Quick {
                                failed: "The event was not deleted",
                                closes_editor,
                            },
                        )
                    });
                });
                confirm(
                    Question {
                        title: format!("Delete \"{}\"?", event.title),
                        description,
                        verb: "Delete",
                        guests: has_guests_to_notify(&existing.original),
                    },
                    chosen,
                    Rc::new(|_, _| {}),
                    window,
                    cx,
                );
            },
        );
    }

    /// Sets the signed-in account's reply to an invitation.
    pub(crate) fn reply(
        &mut self,
        event: EventRef,
        response: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.read_then(
            Some(&event.clone()),
            window,
            cx,
            move |view, read, window, cx| {
                let tz = view.model.read(cx).tz;
                let prepared = prepare(read.raw, &event).and_then(|existing| {
                    let mut draft = EventDraft::from_raw(&existing.original, tz)
                        .ok_or("This event has no time takt can read.")?;
                    let me = draft
                        .guests
                        .iter_mut()
                        .find(|guest| guest.is_self)
                        .ok_or("You are not a guest of this event.")?;
                    me.response = response.to_owned();
                    Ok((existing, draft))
                });
                let (existing, draft) = match prepared {
                    Ok(prepared) => prepared,
                    Err(message) => return view.notice(message, window, cx),
                };
                let patch = draft.patch(&existing.original, tz, "");
                if patch.as_object().is_none_or(|fields| fields.is_empty()) {
                    return;
                }
                view.send_write(
                    event.account_id.clone(),
                    event.calendar_id.clone(),
                    WriteOp::Update {
                        event_id: existing.event_id,
                        etag: existing.etag,
                        base: base_of(&existing.original, &patch),
                        patch,
                        move_to: None,
                    },
                    false,
                    Pending::Quick {
                        failed: "Your reply was not saved",
                        closes_editor: false,
                    },
                );
            },
        );
    }

    /// Sends a dragged event to its new time. With guests to tell, asks first.
    fn move_event(
        &mut self,
        event: EventRef,
        moved: Moved,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.read_then(
            Some(&event.clone()),
            window,
            cx,
            move |view, read, window, cx| {
                let tz = view.model.read(cx).tz;
                let prepared = prepare(read.raw, &event).and_then(|existing| {
                    let mut draft = EventDraft::from_raw(&existing.original, tz)
                        .ok_or("This event has no time takt can read.")?;
                    draft.time = moved.apply(draft.time, tz);
                    Ok((existing, draft))
                });
                let (existing, draft) = match prepared {
                    Ok(prepared) => prepared,
                    Err(message) => {
                        view.drag = None;
                        return view.notice(message, window, cx);
                    }
                };
                let patch = draft.patch(&existing.original, tz, "");
                if patch.as_object().is_none_or(|fields| fields.is_empty()) {
                    view.drag = None;
                    cx.notify();
                    return;
                }
                let guests = has_guests_to_notify(&existing.original);
                let op = WriteOp::Update {
                    event_id: existing.event_id,
                    etag: existing.etag,
                    base: base_of(&existing.original, &patch),
                    patch,
                    move_to: None,
                };
                let (account_id, calendar_id) =
                    (event.account_id.clone(), event.calendar_id.clone());
                let pending = || Pending::Quick {
                    failed: "The event was not moved",
                    closes_editor: false,
                };
                if !guests {
                    return view.send_write(account_id, calendar_id, op, false, pending());
                }
                let this = cx.entity().downgrade();
                let chosen: Chosen = {
                    let this = this.clone();
                    Rc::new(move |notify, _, cx| {
                        let _ = this.update(cx, |view, _| {
                            view.send_write(
                                account_id.clone(),
                                calendar_id.clone(),
                                op.clone(),
                                notify,
                                pending(),
                            )
                        });
                    })
                };
                let cancelled = Rc::new(move |_: &mut Window, cx: &mut App| {
                    let _ = this.update(cx, |view, cx| {
                        view.drag = None;
                        cx.notify();
                    });
                });
                confirm(
                    Question {
                        title: format!("Move \"{}\"?", event.title),
                        description: "Guests see the new time either way.",
                        verb: "Move",
                        guests: true,
                    },
                    chosen,
                    cancelled,
                    window,
                    cx,
                );
            },
        );
    }

    // ---- Dragging ----

    /// The wall-clock time under `position` in the time grid.
    fn time_at(&self, position: Point<Pixels>, cx: &App) -> Option<NaiveDateTime> {
        let bounds = self.time_area.get()?;
        let days = &self.model.read(cx).snapshot.grid.days;
        let width = f32::from(bounds.size.width);
        if days.is_empty() || width <= 0. {
            return None;
        }
        let across = f32::from(position.x - bounds.origin.x) / width * days.len() as f32;
        let column = (across.floor() as isize).clamp(0, days.len() as isize - 1) as usize;
        let down = f32::from(position.y - bounds.origin.y) / HOUR_HEIGHT * 60.;
        let minutes = down.clamp(0., 24. * 60.) as i64;
        Some(days[column].date.and_time(NaiveTime::MIN) + TimeDelta::minutes(minutes))
    }

    /// The day under `position` in the month, or in a week's all-day strip.
    fn day_at(&self, position: Point<Pixels>, cx: &App) -> Option<NaiveDate> {
        let bounds = self.day_area.get()?;
        let snapshot = &self.model.read(cx).snapshot;
        let rows: Vec<Vec<NaiveDate>> = if snapshot.view == ViewKind::Month {
            snapshot
                .month
                .weeks
                .iter()
                .map(|w| w.days.clone())
                .collect()
        } else {
            vec![snapshot.grid.days.iter().map(|d| d.date).collect()]
        };
        let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        if rows.is_empty() || width <= 0. || height <= 0. {
            return None;
        }
        let down = f32::from(position.y - bounds.origin.y) / height * rows.len() as f32;
        let row = &rows[(down.floor() as isize).clamp(0, rows.len() as isize - 1) as usize];
        if row.is_empty() {
            return None;
        }
        let across = f32::from(position.x - bounds.origin.x) / width * row.len() as f32;
        Some(row[(across.floor() as isize).clamp(0, row.len() as isize - 1) as usize])
    }

    /// The mouse button went down on an event, or with `event` `None` on an
    /// empty part of the time grid. Elements under the pointer all hear the
    /// press, innermost first; the first one takes it. A double click edits
    /// the event or starts a new one; a single press may become a drag.
    pub(crate) fn press(
        &mut self,
        event: Option<EventRef>,
        mode: DragMode,
        position: Point<Pixels>,
        clicks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if std::mem::replace(&mut self.press_claimed, true) {
            return;
        }
        match (clicks, event) {
            (2, Some(event)) => self.edit_event(event, window, cx),
            (2, None) => {
                let Some(at) = self.time_at(position, cx) else {
                    return;
                };
                // An hour from the half hour the pointer is in.
                let minutes = i64::from(at.time().num_seconds_from_midnight()) / 60 / 30 * 30;
                let start = at.date().and_time(NaiveTime::MIN) + TimeDelta::minutes(minutes);
                let zone = self.model.read(cx).tz;
                self.new_event(
                    Some(DraftTime::Timed {
                        start,
                        end: start + TimeDelta::hours(1),
                        zone,
                    }),
                    window,
                    cx,
                );
            }
            (_, event) => self.begin_drag(event, mode, position, cx),
        }
    }

    /// The mouse button went down on a day of the month, outside any event.
    pub(crate) fn press_day(
        &mut self,
        date: NaiveDate,
        clicks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if std::mem::replace(&mut self.press_claimed, true) || clicks != 2 {
            return;
        }
        self.new_event(
            Some(DraftTime::AllDay {
                first: date,
                last: date,
            }),
            window,
            cx,
        );
    }

    /// Nothing shows until the pointer moves.
    fn begin_drag(
        &mut self,
        event: Option<EventRef>,
        mode: DragMode,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.suppress_click = false;
        // A resize handle or an event already began one for this press.
        if self.drag.as_ref().is_some_and(|drag| !drag.settling) {
            return;
        }
        if let Some(event) = &event
            && !self.is_writable(&event.account_id, &event.calendar_id, cx)
        {
            return;
        }
        let tz = self.model.read(cx).tz;
        let start = match mode {
            DragMode::Days => self
                .day_at(position, cx)
                .map(|day| (day.and_time(NaiveTime::MIN), Preview::Day(day))),
            DragMode::Create => self.time_at(position, cx).map(|at| {
                let at = snap(at - TimeDelta::minutes(SNAP_MINUTES / 2));
                let end = at + TimeDelta::minutes(SNAP_MINUTES);
                (at, Preview::Timed { start: at, end })
            }),
            DragMode::Move | DragMode::Resize => match event.as_ref().map(|e| (e.start, e.end)) {
                Some((When::At(start), When::At(end))) => {
                    let local = |t: DateTime<Utc>| t.with_timezone(&tz).naive_local();
                    self.time_at(position, cx).map(|at| {
                        (
                            at,
                            Preview::Timed {
                                start: local(start),
                                end: local(end),
                            },
                        )
                    })
                }
                _ => None,
            },
        };
        let Some((anchor, initial)) = start else {
            return;
        };
        self.drag = Some(Drag {
            event,
            mode,
            anchor,
            initial,
            preview: initial,
            moved: false,
            settling: false,
            answered: false,
        });
    }

    /// The pointer moved. `pressed` is false when the button was let go
    /// somewhere the release did not reach takt.
    pub(super) fn drag_to(
        &mut self,
        position: Point<Pixels>,
        pressed: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = &self.drag else {
            return;
        };
        if drag.settling {
            return;
        }
        if !pressed {
            self.drag = None;
            cx.notify();
            return;
        }
        let quarter = TimeDelta::minutes(SNAP_MINUTES);
        let preview = match (drag.mode, drag.initial) {
            (DragMode::Days, _) => self.day_at(position, cx).map(Preview::Day),
            (DragMode::Move, Preview::Timed { start, end }) => {
                self.time_at(position, cx).map(|at| {
                    let moved = snap(start + (at - drag.anchor));
                    Preview::Timed {
                        start: moved,
                        end: moved + (end - start),
                    }
                })
            }
            (DragMode::Resize, Preview::Timed { start, .. }) => {
                self.time_at(position, cx).map(|at| Preview::Timed {
                    start,
                    end: snap(at).max(start + quarter),
                })
            }
            (DragMode::Create, _) => self.time_at(position, cx).map(|at| {
                let at = snap(at);
                let (start, end) = if at >= drag.anchor {
                    (drag.anchor, at)
                } else {
                    (at, drag.anchor)
                };
                Preview::Timed {
                    start,
                    end: end.max(start + quarter),
                }
            }),
            _ => None,
        };
        let (Some(preview), Some(drag)) = (preview, self.drag.as_mut()) else {
            return;
        };
        if drag.preview != preview {
            drag.preview = preview;
            drag.moved = preview != drag.initial;
            cx.notify();
        }
    }

    /// The button came up: a drag that went somewhere is carried out.
    pub(super) fn end_drag(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.press_claimed = false;
        let Some(drag) = &mut self.drag else {
            return;
        };
        if drag.settling {
            return;
        }
        if !drag.moved {
            // A plain click; the element's own click handler deals with it.
            self.drag = None;
            cx.notify();
            return;
        }
        let (mode, preview, event, anchor) =
            (drag.mode, drag.preview, drag.event.clone(), drag.anchor);
        drag.settling = true;
        self.suppress_click = true;
        let tz = self.model.read(cx).tz;
        match (mode, preview, event) {
            (DragMode::Create, Preview::Timed { start, end }, _) => {
                self.drag = None;
                self.new_event(
                    Some(DraftTime::Timed {
                        start,
                        end,
                        zone: tz,
                    }),
                    window,
                    cx,
                );
            }
            (DragMode::Days, Preview::Day(day), Some(event)) => {
                let days = (day - anchor.date()).num_days();
                self.move_event(event, Moved::Days(days), window, cx);
            }
            (DragMode::Move | DragMode::Resize, Preview::Timed { start, end }, Some(event)) => {
                let instant = |t| local_instant(t, tz).with_timezone(&Utc);
                self.move_event(event, Moved::To(instant(start), instant(end)), window, cx);
            }
            _ => self.drag = None,
        }
        cx.notify();
    }

    /// Whether the click that ends a drag should be ignored. Asking uses it up.
    pub(super) fn take_suppressed_click(&mut self) -> bool {
        std::mem::take(&mut self.suppress_click)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;
    use serde_json::json;
    use takt_store::{AccountRow, AccountStatus};

    // Not `super::*`: that brings in GPUI's `test` macro, which shadows `#[test]`.
    use super::{EventRef, Moved, calendar_choices, prepare, snap};
    use crate::draft::DraftTime;
    use crate::model::Snapshot;
    use takt_store::{CalendarRow, When};

    fn at(day: u32, hour: u32, minute: u32) -> chrono::NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, day)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    fn event(event_id: &str, instance_id: &str) -> EventRef {
        let start = chrono::Utc
            .with_ymd_and_hms(2026, 10, 27, 12, 0, 0)
            .unwrap();
        EventRef {
            account_id: "acc".into(),
            calendar_id: "cal".into(),
            event_id: event_id.into(),
            instance_id: instance_id.into(),
            start: When::At(start),
            end: When::At(start + chrono::TimeDelta::hours(1)),
            title: "Lunch".into(),
            color: 0,
        }
    }

    #[test]
    fn snapping_rounds_to_the_nearest_quarter() {
        assert_eq!(snap(at(20, 9, 7)), at(20, 9, 0));
        assert_eq!(snap(at(20, 9, 8)), at(20, 9, 15));
        assert_eq!(snap(at(20, 23, 55)), at(21, 0, 0));
    }

    #[test]
    fn a_single_event_is_written_by_its_own_id_and_etag() {
        let raw = json!({"id": "e1", "etag": "\"3\"", "summary": "Lunch"});
        let existing = prepare(Some(raw.clone()), &event("e1", "e1")).unwrap();
        assert_eq!(existing.event_id, "e1");
        assert_eq!(existing.etag.as_deref(), Some("\"3\""));
        assert_eq!(existing.original, raw);
        assert!(existing.instance.is_none());
        assert!(prepare(None, &event("e1", "e1")).is_err());
    }

    #[test]
    fn a_generated_occurrence_is_written_by_its_instance_id_without_an_etag() {
        let series = json!({
            "id": "m", "etag": "\"3\"", "summary": "Lunch",
            "start": {"dateTime": "2026-10-20T12:00:00Z"}, "end": {"dateTime": "2026-10-20T13:00:00Z"},
            "recurrence": ["RRULE:FREQ=WEEKLY"]
        });
        let occurrence = event("m", "m_20261027T120000Z");
        let existing = prepare(Some(series.clone()), &occurrence).unwrap();
        assert_eq!(existing.event_id, "m_20261027T120000Z");
        assert_eq!(existing.etag, None);
        assert_eq!(
            existing.original["start"]["dateTime"],
            "2026-10-27T12:00:00Z"
        );
        assert!(existing.original.get("recurrence").is_none());
        let instance = existing.instance.unwrap();
        assert_eq!(instance.series_id, "m");
        assert_eq!(instance.original_start, occurrence.start);

        // A series takt could not expand is shown whole and is not editable here.
        assert!(prepare(Some(series), &event("m", "m")).is_err());
    }

    #[test]
    fn a_changed_occurrence_is_an_event_of_its_own() {
        let exception = json!({
            "id": "m_20261027T120000Z", "etag": "\"5\"", "recurringEventId": "m",
            "start": {"dateTime": "2026-10-27T14:00:00Z"}, "end": {"dateTime": "2026-10-27T15:00:00Z"}
        });
        let shown = event("m_20261027T120000Z", "m_20261027T120000Z");
        let existing = prepare(Some(exception), &shown).unwrap();
        assert_eq!(existing.etag.as_deref(), Some("\"5\""));
        assert!(existing.instance.is_none());
    }

    #[test]
    fn a_day_move_keeps_the_time_of_day() {
        let zone = chrono_tz::UTC;
        let timed = DraftTime::Timed {
            start: at(20, 9, 0),
            end: at(20, 10, 0),
            zone,
        };
        assert_eq!(
            Moved::Days(2).apply(timed, zone),
            DraftTime::Timed {
                start: at(22, 9, 0),
                end: at(22, 10, 0),
                zone
            }
        );
        let all_day = DraftTime::AllDay {
            first: at(20, 0, 0).date(),
            last: at(21, 0, 0).date(),
        };
        assert_eq!(
            Moved::Days(-1).apply(all_day, zone),
            DraftTime::AllDay {
                first: at(19, 0, 0).date(),
                last: at(20, 0, 0).date()
            }
        );
    }

    fn calendar(account: &str, id: &str, name: &str, role: &str) -> CalendarRow {
        CalendarRow {
            account_id: account.into(),
            calendar_id: id.into(),
            summary: name.into(),
            time_zone: None,
            background_color: Some("#9fe1e7".into()),
            foreground_color: None,
            access_role: Some(role.into()),
            selected: true,
            hidden: false,
            primary: id == "me",
            visible: true,
        }
    }

    #[test]
    fn only_writable_calendars_are_offered_and_labels_are_unique() {
        let account = |id: &str| AccountRow {
            account_id: id.into(),
            email: format!("{id}@example.com"),
            status: AccountStatus::Ok,
        };
        let snapshot = Snapshot {
            accounts: vec![account("a"), account("b")],
            calendars: vec![
                calendar("a", "me", "Me", "owner"),
                calendar("a", "team", "Team", "writer"),
                calendar("a", "holidays", "Holidays", "reader"),
                calendar("a", "team2", "Team", "owner"),
                calendar("b", "me", "Me", "owner"),
            ],
            ..Default::default()
        };
        let labels = |account: Option<&str>| -> Vec<String> {
            calendar_choices(&snapshot, account)
                .into_iter()
                .map(|c| c.label)
                .collect()
        };
        assert_eq!(labels(Some("a")), ["Me", "Team", "Team (team2)"]);
        assert_eq!(
            labels(None),
            [
                "Me · a@example.com",
                "Team · a@example.com",
                "Team · a@example.com (team2)",
                "Me · b@example.com"
            ]
        );
        let choices = calendar_choices(&snapshot, Some("a"));
        assert!(choices[0].primary && !choices[1].primary);
        assert_eq!(choices[0].color, 0x9fe1e7);
    }
}
