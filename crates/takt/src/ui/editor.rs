//! The event editor: a panel beside the calendar for a new event or an
//! existing one. It owns the inputs, reads them into an [`EventDraft`] when
//! saving, and emits the write for the main view to send.

use std::collections::HashMap;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use chrono_tz::Tz;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{
    Enter, Escape, Input, InputEvent, InputState, Textarea, TextareaState,
};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IndexPath, Selectable as _, Sizable as _, StyledExt as _,
    WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use takt_sync::{InstanceOf, WriteOp};

use crate::draft::{
    AutoDecline, DraftTime, EventDraft, Guest, Kind, Meet, Place, Reminder, ReminderMethod,
    Reminders, base_of, changed_parts, looks_like_email,
};
use crate::entry::{
    Unit, format_date, format_time, join_minutes, parse_date, parse_time, split_minutes,
};
use crate::keys::{CancelEdit, EDITOR_CONTEXT, SaveEdit};

pub const EDITOR_WIDTH: f32 = 420.;

/// A calendar an event can be saved to.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarChoice {
    pub account_id: String,
    pub calendar_id: String,
    /// What the calendar picker shows; unique among the choices.
    pub label: String,
    pub account_email: String,
    pub color: u32,
    pub primary: bool,
}

/// The event being changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Existing {
    /// The ID writes go to: the event's own, or an occurrence's instance ID.
    pub event_id: String,
    pub etag: Option<String>,
    /// The JSON the draft was read from; a save sends what differs from it.
    pub original: Value,
    /// Set for an occurrence of a series.
    pub instance: Option<InstanceOf>,
}

pub struct EditorSetup {
    /// `None` for a new event.
    pub existing: Option<Existing>,
    pub draft: EventDraft,
    /// The calendars to offer. For an existing event, its own account's.
    pub calendars: Vec<CalendarChoice>,
    /// Which of `calendars` the event is in.
    pub calendar: usize,
    /// Each account's event colours: (colour ID, RGB).
    pub colors: HashMap<String, Vec<(String, u32)>>,
    pub tz: Tz,
    pub use_24h: bool,
    pub today: NaiveDate,
}

/// What a save asks sync to do.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveRequest {
    pub account_id: String,
    pub calendar_id: String,
    pub op: WriteOp,
    pub notify_guests: bool,
}

pub enum EditorEvent {
    Save(SaveRequest),
    /// Closed without a write: cancelled, or saved with nothing changed.
    Close,
    /// The Delete button, for the event being edited.
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum KindTag {
    Event,
    Focus,
    Away,
    Place,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlaceTag {
    Home,
    Office,
    Custom,
}

pub(crate) struct ReminderRow {
    pub method: ReminderMethod,
    pub amount: Entity<InputState>,
    pub unit: Unit,
}

pub struct EventEditor {
    pub(crate) existing: Option<Existing>,
    calendars: Vec<CalendarChoice>,
    /// The calendar the event was in when the editor opened.
    home: usize,
    pub(crate) calendar: usize,
    calendar_select: Entity<SelectState<Vec<SharedString>>>,
    colors: HashMap<String, Vec<(String, u32)>>,
    tz: Tz,
    use_24h: bool,
    today: NaiveDate,

    pub(crate) title: Entity<InputState>,
    pub(crate) location: Entity<InputState>,
    pub(crate) description: Entity<TextareaState>,
    pub(crate) start_date: Entity<InputState>,
    pub(crate) start_time: Entity<InputState>,
    pub(crate) end_date: Entity<InputState>,
    pub(crate) end_time: Entity<InputState>,
    pub(crate) zone: Entity<InputState>,
    pub(crate) guest: Entity<InputState>,
    message: Entity<InputState>,
    place_label: Entity<InputState>,

    pub(crate) all_day: bool,
    pub(crate) color_id: Option<String>,
    pub(crate) guests: Vec<Guest>,
    pub(crate) default_reminders: bool,
    pub(crate) reminders: Vec<ReminderRow>,
    pub(crate) meet: Meet,
    kind: KindTag,
    /// A type takt has no fields for, kept as it is.
    other_kind: Option<String>,
    decline: AutoDecline,
    do_not_disturb: bool,
    place: PlaceTag,
    pub(crate) notify: bool,

    /// The start the end was last lined up with: moving the start moves the
    /// end along, keeping the length.
    last_start: Option<NaiveDateTime>,
    /// The draft as the editor opened, to tell whether anything was typed.
    initial: EventDraft,
    pub(crate) problem: Option<String>,
    pub(crate) notice: Option<String>,
    pub(crate) saving: bool,
    requests: u32,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EditorEvent> for EventEditor {}

impl Focusable for EventEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn kind_parts(kind: &Kind) -> (KindTag, AutoDecline, String, bool, PlaceTag, String) {
    let none = (
        KindTag::Event,
        AutoDecline::None,
        String::new(),
        true,
        PlaceTag::Home,
        String::new(),
    );
    match kind {
        Kind::Event | Kind::Other(_) => none,
        Kind::FocusTime {
            decline,
            message,
            do_not_disturb,
        } => (
            KindTag::Focus,
            *decline,
            message.clone(),
            *do_not_disturb,
            none.4,
            none.5,
        ),
        Kind::OutOfOffice { decline, message } => (
            KindTag::Away,
            *decline,
            message.clone(),
            none.3,
            none.4,
            none.5,
        ),
        Kind::WorkingLocation(place) => {
            let (tag, label) = match place {
                Place::Home => (PlaceTag::Home, String::new()),
                Place::Office(label) => (PlaceTag::Office, label.clone()),
                Place::Custom(label) => (PlaceTag::Custom, label.clone()),
            };
            (KindTag::Place, none.1, none.2, none.3, tag, label)
        }
    }
}

impl EventEditor {
    pub fn new(setup: EditorSetup, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let EditorSetup {
            existing,
            draft,
            calendars,
            calendar,
            colors,
            tz,
            use_24h,
            today,
        } = setup;
        let input = |value: String, placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let (start, end, zone) = match draft.time {
            DraftTime::AllDay { first, last } => (
                first.and_time(NaiveTime::MIN),
                last.and_time(NaiveTime::MIN),
                None,
            ),
            DraftTime::Timed { start, end, zone } => (start, end, Some(zone)),
        };
        let time = |t: NaiveDateTime| {
            if draft.time.is_all_day() {
                String::new()
            } else {
                format_time(t.time(), use_24h)
            }
        };
        let (kind, decline, message, do_not_disturb, place, place_label) = kind_parts(&draft.kind);
        let (default_reminders, reminder_list) = match &draft.reminders {
            Reminders::Default => (true, Vec::new()),
            Reminders::Custom(list) => (false, list.clone()),
        };
        let reminders = reminder_list
            .iter()
            .map(|reminder| {
                let (amount, unit) = split_minutes(reminder.minutes);
                ReminderRow {
                    method: reminder.method,
                    amount: input(amount.to_string(), "10", window, cx),
                    unit,
                }
            })
            .collect();

        let title = input(draft.summary.clone(), "Add title", window, cx);
        let guest = input(String::new(), "Add guest email", window, cx);
        let start_date = input(format_date(start.date()), "Date", window, cx);
        let start_time = input(time(start), "Time", window, cx);
        let description = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .placeholder("Add description")
                .default_value(draft.description.clone());
            state.set_auto_grow(3, 12, cx);
            state
        });
        let labels: Vec<SharedString> = calendars
            .iter()
            .map(|c| SharedString::from(c.label.clone()))
            .collect();
        let calendar_select =
            cx.new(|cx| SelectState::new(labels, Some(IndexPath::new(calendar)), window, cx));

        let subscriptions = vec![
            cx.subscribe_in(&title, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.save(window, cx);
                }
            }),
            cx.subscribe_in(&guest, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.add_typed_guest(window, cx);
                }
            }),
            cx.subscribe_in(
                &start_date,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        this.align_end(window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &start_time,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                        this.align_end(window, cx);
                    }
                },
            ),
            cx.subscribe_in(
                &calendar_select,
                window,
                |this, _, event: &SelectEvent<Vec<SharedString>>, _, cx| {
                    let SelectEvent::Confirm(Some(label)) = event else {
                        return;
                    };
                    if let Some(ix) = this.calendars.iter().position(|c| c.label == label.as_ref())
                    {
                        this.set_calendar(ix, cx);
                    }
                },
            ),
        ];

        Self {
            existing,
            home: calendar,
            calendar,
            calendar_select,
            calendars,
            colors,
            tz,
            use_24h,
            today,
            location: input(draft.location.clone(), "Add location", window, cx),
            description,
            end_date: input(format_date(end.date()), "Date", window, cx),
            end_time: input(time(end), "Time", window, cx),
            zone: input(
                zone.map(|z| z.name().to_owned()).unwrap_or_default(),
                tz.name(),
                window,
                cx,
            ),
            message: input(message, "Decline message", window, cx),
            place_label: input(place_label, "Where", window, cx),
            title,
            guest,
            start_date,
            start_time,
            all_day: draft.time.is_all_day(),
            color_id: draft.color_id.clone(),
            guests: draft.guests.clone(),
            default_reminders,
            reminders,
            meet: draft.meet.clone(),
            kind,
            other_kind: match &draft.kind {
                Kind::Other(name) => Some(name.clone()),
                _ => None,
            },
            decline,
            do_not_disturb,
            place,
            notify: true,
            last_start: (!draft.time.is_all_day()).then_some(start),
            initial: draft,
            problem: None,
            notice: None,
            saving: false,
            requests: 0,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    pub fn is_new(&self) -> bool {
        self.existing.is_none()
    }

    pub fn focus_title(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.title.update(cx, |title, cx| title.focus(window, cx));
    }

    pub(crate) fn set_calendar(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.calendars.len() || ix == self.calendar {
            return;
        }
        let account_changed = self.calendars[ix].account_id != self.calendars[self.calendar].account_id;
        self.calendar = ix;
        if account_changed {
            // Colour IDs belong to an account's palette.
            self.color_id = None;
        }
        cx.notify();
    }

    fn value(&self, input: &Entity<InputState>, cx: &App) -> String {
        input.read(cx).value().trim().to_owned()
    }

    fn date(&self, input: &Entity<InputState>, what: &str, cx: &App) -> Result<NaiveDate, String> {
        let text = self.value(input, cx);
        parse_date(&text, self.today)
            .ok_or_else(|| format!("\"{text}\" is not a date takt can read ({what})."))
    }

    fn time(&self, input: &Entity<InputState>, what: &str, cx: &App) -> Result<NaiveTime, String> {
        let text = self.value(input, cx);
        parse_time(&text).ok_or_else(|| format!("\"{text}\" is not a time takt can read ({what})."))
    }

    /// The start as typed, when it reads as a date and a time.
    fn typed_start(&self, cx: &App) -> Option<NaiveDateTime> {
        let date = self.date(&self.start_date, "start", cx).ok()?;
        let time = self.time(&self.start_time, "start", cx).ok()?;
        Some(date.and_time(time))
    }

    /// Moves the end by as much as the start moved since the last call.
    fn align_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.all_day {
            return;
        }
        let Some(start) = self.typed_start(cx) else {
            return;
        };
        let moved = self.last_start.map_or(TimeDelta::zero(), |last| start - last);
        self.last_start = Some(start);
        if moved.is_zero() {
            return;
        }
        let end = self
            .date(&self.end_date, "end", cx)
            .ok()
            .zip(self.time(&self.end_time, "end", cx).ok())
            .map(|(date, time)| date.and_time(time) + moved);
        if let Some(end) = end {
            let use_24h = self.use_24h;
            self.end_date.update(cx, |input, cx| {
                input.set_value(format_date(end.date()), window, cx)
            });
            self.end_time.update(cx, |input, cx| {
                input.set_value(format_time(end.time(), use_24h), window, cx)
            });
        }
    }

    pub(crate) fn set_all_day(&mut self, all_day: bool, window: &mut Window, cx: &mut Context<Self>) {
        if all_day == self.all_day {
            return;
        }
        self.all_day = all_day;
        if !all_day {
            // Coming back from all-day: an hour from 09:00, or the old times.
            let blank = self.value(&self.start_time, cx).is_empty();
            if blank {
                let use_24h = self.use_24h;
                let at = |hour| {
                    format_time(
                        NaiveTime::from_hms_opt(hour, 0, 0).unwrap_or_default(),
                        use_24h,
                    )
                };
                self.start_time
                    .update(cx, |input, cx| input.set_value(at(9), window, cx));
                self.end_time
                    .update(cx, |input, cx| input.set_value(at(10), window, cx));
            }
            self.last_start = self.typed_start(cx);
        }
        cx.notify();
    }

    fn add_typed_guest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.value(&self.guest, cx);
        // Several addresses can be pasted at once.
        let emails: Vec<&str> = typed
            .split([',', ';', ' '])
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .collect();
        if emails.is_empty() {
            return;
        }
        if let Some(bad) = emails.iter().find(|e| !looks_like_email(e)) {
            self.problem = Some(format!("\"{bad}\" is not an email address."));
            cx.notify();
            return;
        }
        let choice = &self.calendars[self.calendar];
        // Like the web: the first guest makes the organizer a guest too.
        if self.guests.is_empty() && choice.primary {
            let mut me = Guest::new(&choice.account_email);
            me.is_self = true;
            me.organizer = true;
            me.response = "accepted".into();
            self.guests.push(me);
        }
        for email in emails {
            let known = self
                .guests
                .iter()
                .any(|g| g.email.eq_ignore_ascii_case(email));
            if !known {
                self.guests.push(Guest::new(email));
            }
        }
        self.problem = None;
        self.guest
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    fn add_reminder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.default_reminders = false;
        let amount = cx.new(|cx| InputState::new(window, cx).default_value("10"));
        self.reminders.push(ReminderRow {
            method: ReminderMethod::Popup,
            amount,
            unit: Unit::Minutes,
        });
        cx.notify();
    }

    fn kind(&self, cx: &App) -> Kind {
        if let Some(other) = &self.other_kind {
            return Kind::Other(other.clone());
        }
        match self.kind {
            KindTag::Event => Kind::Event,
            KindTag::Focus => Kind::FocusTime {
                decline: self.decline,
                message: self.value(&self.message, cx),
                do_not_disturb: self.do_not_disturb,
            },
            KindTag::Away => Kind::OutOfOffice {
                decline: self.decline,
                message: self.value(&self.message, cx),
            },
            KindTag::Place => Kind::WorkingLocation(match self.place {
                PlaceTag::Home => Place::Home,
                PlaceTag::Office => Place::Office(self.value(&self.place_label, cx)),
                PlaceTag::Custom => Place::Custom(self.value(&self.place_label, cx)),
            }),
        }
    }

    /// The inputs as a draft, or what is wrong with them.
    pub(crate) fn read(&self, cx: &App) -> Result<EventDraft, String> {
        let first = self.date(&self.start_date, "start", cx)?;
        let last = self.date(&self.end_date, "end", cx)?;
        let time = if self.all_day {
            DraftTime::AllDay { first, last }
        } else {
            let zone_name = self.value(&self.zone, cx);
            let zone = if zone_name.is_empty() {
                self.tz
            } else {
                zone_name
                    .parse::<Tz>()
                    .map_err(|_| format!("\"{zone_name}\" is not a time zone name like Europe/Berlin."))?
            };
            DraftTime::Timed {
                start: first.and_time(self.time(&self.start_time, "start", cx)?),
                end: last.and_time(self.time(&self.end_time, "end", cx)?),
                zone,
            }
        };
        let reminders = if self.default_reminders {
            Reminders::Default
        } else {
            let list = self
                .reminders
                .iter()
                .map(|row| {
                    let amount = self.value(&row.amount, cx);
                    join_minutes(&amount, row.unit)
                        .map(|minutes| Reminder {
                            method: row.method,
                            minutes,
                        })
                        .ok_or_else(|| {
                            format!(
                                "A reminder of \"{amount}\" {} is not possible (at most 4 weeks).",
                                row.unit.label()
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Reminders::Custom(list)
        };
        let draft = EventDraft {
            summary: self.value(&self.title, cx),
            location: self.value(&self.location, cx),
            description: self.description.read(cx).value().trim_end().to_owned(),
            time,
            color_id: self.color_id.clone(),
            guests: self.guests.clone(),
            reminders,
            meet: self.meet.clone(),
            kind: self.kind(cx),
        };
        match draft.problem() {
            Some(problem) => Err(problem),
            None => Ok(draft),
        }
    }

    pub(crate) fn is_dirty(&self, cx: &App) -> bool {
        self.calendar != self.home
            || !self.value(&self.guest, cx).is_empty()
            || self.read(cx).map_or(true, |draft| draft != self.initial)
    }

    /// Reads the inputs and asks for the write. With nothing changed on an
    /// existing event, just closes.
    pub(crate) fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.align_end(window, cx);
        self.add_typed_guest(window, cx);
        if !self.value(&self.guest, cx).is_empty() {
            // The typed guest was not an address; the problem is already set.
            return;
        }
        let draft = match self.read(cx) {
            Ok(draft) => draft,
            Err(problem) => {
                self.problem = Some(problem);
                cx.notify();
                return;
            }
        };
        self.requests += 1;
        // Names a Meet link request; Google ignores a repeat of the same one.
        let request_id = format!(
            "takt-{:x}-{}",
            chrono::Utc::now().timestamp_micros(),
            self.requests
        );
        let chosen = &self.calendars[self.calendar];
        let notify_guests = self.notify && draft.guests.iter().any(|g| !g.is_self);
        let request = match &self.existing {
            None => SaveRequest {
                account_id: chosen.account_id.clone(),
                calendar_id: chosen.calendar_id.clone(),
                op: WriteOp::Create {
                    body: draft.insert_body(&request_id),
                },
                notify_guests,
            },
            Some(existing) => {
                let home = &self.calendars[self.home];
                let patch = draft.patch(&existing.original, self.tz, &request_id);
                let move_to = (self.calendar != self.home).then(|| chosen.calendar_id.clone());
                if move_to.is_none() && patch.as_object().is_none_or(|fields| fields.is_empty()) {
                    cx.emit(EditorEvent::Close);
                    return;
                }
                SaveRequest {
                    account_id: home.account_id.clone(),
                    calendar_id: home.calendar_id.clone(),
                    op: WriteOp::Update {
                        event_id: existing.event_id.clone(),
                        etag: existing.etag.clone(),
                        base: base_of(&existing.original, &patch),
                        patch,
                        move_to,
                    },
                    notify_guests,
                }
            }
        };
        self.problem = None;
        self.saving = true;
        cx.emit(EditorEvent::Save(request));
        cx.notify();
    }

    /// The write was refused; the inputs stay as they are.
    pub fn write_failed(&mut self, message: String, cx: &mut Context<Self>) {
        self.saving = false;
        self.problem = Some(message);
        cx.notify();
    }

    /// The event changed in Google in a field this edit also changes. The
    /// inputs stay; the next save is compared with `current`, Google's
    /// version, and so puts the typed values over it.
    pub fn rebase(&mut self, current: Value, cx: &mut Context<Self>) {
        self.saving = false;
        let Some(existing) = &mut self.existing else {
            return;
        };
        let parts = changed_parts(&existing.original, &current, self.tz);
        self.notice = Some(if parts.is_empty() {
            "This event changed in Google while you were editing. Save again to keep your version."
                .into()
        } else {
            format!(
                "Changed in Google while you were editing: {}. Save again to put your version over it, or cancel to keep Google's.",
                parts.join(", ")
            )
        });
        existing.etag = current["etag"].as_str().map(str::to_owned);
        existing.original = current;
        cx.notify();
    }

    /// Closes, asking first when something was typed.
    pub(crate) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_dirty(cx) {
            cx.emit(EditorEvent::Close);
            return;
        }
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let this = this.clone();
            dialog
                .title("Discard your changes?")
                .description("Nothing has been sent to Google.")
                .ok_text("Discard")
                .cancel_text("Keep editing")
                .confirm()
                .on_ok(move |_, _, cx| {
                    let _ = this.update(cx, |_, cx| cx.emit(EditorEvent::Close));
                    true
                })
        });
    }

    fn on_enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        // Before the focused input sees it, so a description gets no newline.
        if action.secondary {
            cx.stop_propagation();
            self.save(window, cx);
        }
    }

    fn header(&self) -> &'static str {
        match &self.existing {
            None => "New event",
            Some(existing) if existing.instance.is_some() => "Edit this occurrence",
            Some(_) => "Edit event",
        }
    }

    /// Notes about what a save will and will not do.
    fn remarks(&self) -> Vec<String> {
        let mut remarks = Vec::new();
        let Some(existing) = &self.existing else {
            return remarks;
        };
        let raw = &existing.original;
        if existing.instance.is_some() || raw["recurringEventId"].is_string() {
            remarks.push(
                "Part of a repeating event. Changes apply to this occurrence only.".into(),
            );
        }
        if raw["attendees"].is_array() && raw["organizer"]["self"].as_bool() != Some(true) {
            remarks.push(
                "You are a guest here. Apart from your reply, colour and reminders, changes stay in your own copy."
                    .into(),
            );
        }
        if raw["description"].as_str().is_some_and(|d| d.contains('<')) {
            remarks.push("The description is HTML and is edited as such.".into());
        }
        remarks
    }
}

fn label(text: &'static str, cx: &App) -> Div {
    div()
        .text_xs()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

fn section(text: &'static str, cx: &App) -> Div {
    v_flex().gap_1().child(label(text, cx))
}

/// A small button that is one of several choices.
fn choice(id: impl Into<ElementId>, text: impl Into<SharedString>, chosen: bool) -> Button {
    Button::new(id).ghost().xsmall().label(text).selected(chosen)
}

fn response_label(response: &str) -> &'static str {
    match response {
        "accepted" => "Going",
        "declined" => "Not going",
        "tentative" => "Maybe",
        _ => "No reply yet",
    }
}

impl Render for EventEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let chosen = self.calendars[self.calendar].clone();
        let occurrence = self
            .existing
            .as_ref()
            .is_some_and(|e| e.instance.is_some() || e.original["recurringEventId"].is_string());

        let buttons = h_flex()
            .gap_2()
            .items_center()
            .child(
                Button::new("editor-save")
                    .primary()
                    .small()
                    .label(if self.saving { "Saving…" } else { "Save" })
                    .tooltip("Save (Ctrl+Enter)")
                    .disabled(self.saving)
                    .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
            )
            .child(
                Button::new("editor-cancel")
                    .ghost()
                    .small()
                    .label("Cancel")
                    .tooltip("Cancel (Esc)")
                    .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
            )
            .child(div().flex_1())
            .when(self.existing.is_some(), |row| {
                row.child(
                    Button::new("editor-delete")
                        .ghost()
                        .small()
                        .label("Delete")
                        .disabled(self.saving)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(EditorEvent::Delete))),
                )
            });

        let when = v_flex()
            .gap_1()
            .child(
                Checkbox::new("editor-all-day")
                    .label("All day")
                    .checked(self.all_day)
                    .on_click(cx.listener(|this, checked: &bool, window, cx| {
                        this.set_all_day(*checked, window, cx)
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(36.)).text_sm().child("From"))
                    .child(div().flex_1().child(Input::new(&self.start_date).small()))
                    .when(!self.all_day, |row| {
                        row.child(div().w(px(96.)).child(Input::new(&self.start_time).small()))
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().w(px(36.)).text_sm().child("To"))
                    .child(div().flex_1().child(Input::new(&self.end_date).small()))
                    .when(!self.all_day, |row| {
                        row.child(div().w(px(96.)).child(Input::new(&self.end_time).small()))
                    }),
            )
            .when(!self.all_day, |column| {
                column.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(36.)).text_sm().child("Zone"))
                        .child(div().flex_1().child(Input::new(&self.zone).small())),
                )
            });

        let palette = self
            .colors
            .get(&chosen.account_id)
            .cloned()
            .unwrap_or_default();
        let swatch = |id: Option<String>, color: u32, cx: &mut Context<Self>| {
            let chosen = self.color_id == id;
            let name = id.clone().unwrap_or_else(|| "calendar".into());
            div()
                .id(SharedString::from(format!("editor-color-{name}")))
                .size(px(18.))
                .rounded_full()
                .cursor_pointer()
                .bg(rgb(color))
                .border_2()
                .border_color(if chosen {
                    theme.foreground
                } else {
                    theme.background
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.color_id = id.clone();
                    cx.notify();
                }))
        };
        let mut swatches = vec![swatch(None, chosen.color, cx)];
        for (id, color) in palette {
            swatches.push(swatch(Some(id), color, cx));
        }

        let me = self.guests.iter().position(|g| g.is_self);
        let guest_rows: Vec<AnyElement> = self
            .guests
            .iter()
            .enumerate()
            .map(|(ix, guest)| {
                let who = guest.name.clone().unwrap_or_else(|| guest.email.clone());
                let mut note = response_label(&guest.response).to_owned();
                if guest.organizer {
                    note.push_str(" · organizer");
                }
                h_flex()
                    .gap_1()
                    .items_center()
                    .text_sm()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().child(who))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(note),
                            ),
                    )
                    .child(
                        choice(
                            SharedString::from(format!("editor-guest-optional-{ix}")),
                            "Optional",
                            guest.optional,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(guest) = this.guests.get_mut(ix) {
                                guest.optional = !guest.optional;
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new(SharedString::from(format!("editor-guest-remove-{ix}")))
                            .ghost()
                            .xsmall()
                            .label("×")
                            .tooltip("Remove guest")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if ix < this.guests.len() {
                                    this.guests.remove(ix);
                                }
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            })
            .collect();
        let reply = me.map(|ix| {
            let current = self.guests[ix].response.clone();
            h_flex()
                .gap_1()
                .items_center()
                .child(div().text_sm().child("Going?"))
                .children(
                    [
                        ("accepted", "Yes"),
                        ("tentative", "Maybe"),
                        ("declined", "No"),
                    ]
                    .map(|(response, text)| {
                        choice(
                            SharedString::from(format!("editor-reply-{response}")),
                            text,
                            current == response,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(guest) = this.guests.iter_mut().find(|g| g.is_self) {
                                guest.response = response.into();
                            }
                            cx.notify();
                        }))
                    }),
                )
        });
        let has_others = self.guests.iter().any(|g| !g.is_self);

        let reminder_rows: Vec<AnyElement> = self
            .reminders
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        choice(
                            SharedString::from(format!("editor-reminder-method-{ix}")),
                            match row.method {
                                ReminderMethod::Popup => "Notification",
                                ReminderMethod::Email => "Email",
                            },
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(row) = this.reminders.get_mut(ix) {
                                row.method = match row.method {
                                    ReminderMethod::Popup => ReminderMethod::Email,
                                    ReminderMethod::Email => ReminderMethod::Popup,
                                };
                            }
                            cx.notify();
                        })),
                    )
                    .child(div().w(px(64.)).child(Input::new(&row.amount).small()))
                    .child(
                        choice(
                            SharedString::from(format!("editor-reminder-unit-{ix}")),
                            row.unit.label(),
                            false,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(row) = this.reminders.get_mut(ix) {
                                row.unit = row.unit.next();
                            }
                            cx.notify();
                        })),
                    )
                    .child(div().text_sm().child("before"))
                    .child(div().flex_1())
                    .child(
                        Button::new(SharedString::from(format!("editor-reminder-remove-{ix}")))
                            .ghost()
                            .xsmall()
                            .label("×")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if ix < this.reminders.len() {
                                    this.reminders.remove(ix);
                                }
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            })
            .collect();

        let meet = match &self.meet {
            Meet::None | Meet::Add => Checkbox::new("editor-meet")
                .label("Add a Google Meet link")
                .checked(self.meet == Meet::Add)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.meet = if *checked { Meet::Add } else { Meet::None };
                    cx.notify();
                }))
                .into_any_element(),
            Meet::Existing(link) => h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_1().min_w_0().truncate().text_sm().child(link.clone()))
                .child(
                    Button::new("editor-meet-remove")
                        .ghost()
                        .xsmall()
                        .label("Remove")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.meet = Meet::Remove;
                            cx.notify();
                        })),
                )
                .into_any_element(),
            Meet::Remove => h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("The video call is removed on save."),
                )
                .child(
                    Button::new("editor-meet-keep")
                        .ghost()
                        .xsmall()
                        .label("Keep it")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.meet = this.initial.meet.clone();
                            cx.notify();
                        })),
                )
                .into_any_element(),
        };

        let kind_button = |tag: KindTag, id: &'static str, text: &'static str, cx: &mut Context<Self>| {
            choice(id, text, self.kind == tag).on_click(cx.listener(move |this, _, window, cx| {
                this.kind = tag;
                if tag == KindTag::Away {
                    this.decline = AutoDecline::All;
                }
                if tag == KindTag::Place {
                    this.set_all_day(true, window, cx);
                }
                cx.notify();
            }))
        };
        let decline_button = |mode: AutoDecline, id: &'static str, text: &'static str, cx: &mut Context<Self>| {
            choice(id, text, self.decline == mode).on_click(cx.listener(move |this, _, _, cx| {
                this.decline = mode;
                cx.notify();
            }))
        };
        let place_button = |tag: PlaceTag, id: &'static str, text: &'static str, cx: &mut Context<Self>| {
            choice(id, text, self.place == tag).on_click(cx.listener(move |this, _, _, cx| {
                this.place = tag;
                cx.notify();
            }))
        };
        let kind_options = match self.kind {
            _ if self.other_kind.is_some() => None,
            KindTag::Event => None,
            KindTag::Focus | KindTag::Away => Some(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(div().text_sm().child("Decline meetings:"))
                            .child(decline_button(AutoDecline::None, "editor-decline-none", "None", cx))
                            .child(decline_button(AutoDecline::New, "editor-decline-new", "New ones", cx))
                            .child(decline_button(AutoDecline::All, "editor-decline-all", "All", cx)),
                    )
                    .child(Input::new(&self.message).small())
                    .when(self.kind == KindTag::Focus, |column| {
                        column.child(
                            Checkbox::new("editor-dnd")
                                .label("Do not disturb in Chat")
                                .checked(self.do_not_disturb)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.do_not_disturb = *checked;
                                    cx.notify();
                                })),
                        )
                    }),
            ),
            KindTag::Place => Some(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(place_button(PlaceTag::Home, "editor-place-home", "Home", cx))
                            .child(place_button(PlaceTag::Office, "editor-place-office", "Office", cx))
                            .child(place_button(PlaceTag::Custom, "editor-place-custom", "Somewhere else", cx)),
                    )
                    .when(self.place != PlaceTag::Home, |column| {
                        column.child(Input::new(&self.place_label).small())
                    }),
            ),
        };
        let kinds = self.is_new().then(|| {
            h_flex()
                .gap_1()
                .flex_wrap()
                .child(kind_button(KindTag::Event, "editor-kind-event", "Event", cx))
                .child(kind_button(KindTag::Focus, "editor-kind-focus", "Focus time", cx))
                .child(kind_button(KindTag::Away, "editor-kind-away", "Out of office", cx))
                .child(kind_button(KindTag::Place, "editor-kind-place", "Working location", cx))
        });

        let body = v_flex()
            .gap_3()
            .child(buttons)
            .child(div().text_lg().font_semibold().child(self.header()))
            .when_some(self.notice.clone(), |column, notice| {
                column.child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(theme.warning.opacity(0.15))
                        .text_sm()
                        .child(notice),
                )
            })
            .when_some(self.problem.clone(), |column, problem| {
                column.child(
                    div()
                        .id("editor-problem")
                        .text_sm()
                        .text_color(theme.danger)
                        .child(problem),
                )
            })
            .children(self.remarks().into_iter().map(|remark| {
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(remark)
            }))
            .child(Input::new(&self.title))
            .child(when)
            .child(
                section("Calendar", cx).child(
                    Select::new(&self.calendar_select)
                        .small()
                        .disabled(occurrence || self.calendars.len() < 2),
                ),
            )
            .child(section("Colour", cx).child(h_flex().gap_1().flex_wrap().children(swatches)))
            .child(section("Location", cx).child(Input::new(&self.location).small()))
            .child(section("Video call", cx).child(meet))
            .child(section("Description", cx).child(Textarea::new(&self.description)))
            .child(
                section("Guests", cx)
                    .children(guest_rows)
                    .child(Input::new(&self.guest).small())
                    .children(reply)
                    .when(has_others, |column| {
                        column.child(
                            Checkbox::new("editor-notify")
                                .label("Email guests about this")
                                .checked(self.notify)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.notify = *checked;
                                    cx.notify();
                                })),
                        )
                    }),
            )
            .child(
                section("Reminders", cx)
                    .child(
                        Checkbox::new("editor-default-reminders")
                            .label("The calendar's default reminders")
                            .checked(self.default_reminders)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.default_reminders = *checked;
                                cx.notify();
                            })),
                    )
                    .when(!self.default_reminders, |column| {
                        column.children(reminder_rows)
                    })
                    .child(
                        h_flex().child(
                            Button::new("editor-add-reminder")
                                .ghost()
                                .xsmall()
                                .label("+ Add reminder")
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.add_reminder(window, cx)),
                                ),
                        ),
                    ),
            )
            .when(kinds.is_some() || kind_options.is_some(), |column| {
                column.child(
                    section("Type", cx)
                        .children(kinds)
                        .children(kind_options),
                )
            });

        v_flex()
            .id("editor")
            .key_context(EDITOR_CONTEXT)
            .track_focus(&self.focus)
            .capture_action(cx.listener(Self::on_enter))
            .on_action(cx.listener(|this, _: &Escape, window, cx| this.cancel(window, cx)))
            .on_action(cx.listener(|this, _: &CancelEdit, window, cx| this.cancel(window, cx)))
            .on_action(cx.listener(|this, _: &SaveEdit, window, cx| this.save(window, cx)))
            .w(px(EDITOR_WIDTH))
            .h_full()
            .flex_none()
            .p_4()
            .border_l_1()
            .border_color(theme.border)
            .text_color(theme.foreground)
            .overflow_y_scroll()
            .child(h_flex().w_full().child(body.flex_1().min_w_0()))
    }
}
