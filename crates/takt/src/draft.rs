//! An event while it is being edited, and the JSON to send for it. Pure: no
//! GPUI types.
//!
//! A draft is read from an event's raw JSON and compared with it again when
//! saving, so a patch names only the fields the user changed and everything
//! takt does not show stays as Google has it.

use chrono::{
    DateTime, LocalResult, NaiveDate, NaiveDateTime, SecondsFormat, TimeDelta, TimeZone as _, Utc,
};
use chrono_tz::Tz;
use serde_json::{Map, Value, json};
use takt_store::When;

/// When the event is. Timed events are kept as wall-clock times in `zone`,
/// the zone the event is in (not necessarily the one takt displays).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftTime {
    /// `last` is the last day the event covers (Google's own end date is the
    /// day after).
    AllDay { first: NaiveDate, last: NaiveDate },
    Timed {
        start: NaiveDateTime,
        end: NaiveDateTime,
        zone: Tz,
    },
}

impl DraftTime {
    pub fn is_all_day(&self) -> bool {
        matches!(self, DraftTime::AllDay { .. })
    }

    /// The same event moved to start at `start` and end at `end`.
    pub fn at(self, start: DateTime<Utc>, end: DateTime<Utc>, display: Tz) -> DraftTime {
        let zone = match self {
            DraftTime::Timed { zone, .. } => zone,
            DraftTime::AllDay { .. } => display,
        };
        DraftTime::Timed {
            start: start.with_timezone(&zone).naive_local(),
            end: end.with_timezone(&zone).naive_local(),
            zone,
        }
    }

    /// The start and end as instants; all-day dates begin at midnight in `display`.
    pub fn instants(&self, display: Tz) -> (DateTime<Utc>, DateTime<Utc>) {
        match *self {
            DraftTime::AllDay { first, last } => (
                takt_store::day_start_utc(first, display),
                takt_store::day_start_utc(last + TimeDelta::days(1), display),
            ),
            DraftTime::Timed { start, end, zone } => (
                local_instant(start, zone).with_timezone(&Utc),
                local_instant(end, zone).with_timezone(&Utc),
            ),
        }
    }
}

/// The instant a wall-clock time means in `zone`. A time that happens twice
/// (clocks going back) is the first one; a time that does not exist (clocks
/// going forward) is an hour later.
pub fn local_instant(local: NaiveDateTime, zone: Tz) -> DateTime<Tz> {
    match zone.from_local_datetime(&local) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => t,
        LocalResult::None => zone
            .from_local_datetime(&(local + TimeDelta::hours(1)))
            .earliest()
            .unwrap_or_else(|| zone.from_utc_datetime(&local)),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guest {
    pub email: String,
    pub name: Option<String>,
    pub optional: bool,
    /// `needsAction`, `accepted`, `tentative` or `declined`.
    pub response: String,
    /// The signed-in account's own entry.
    pub is_self: bool,
    pub organizer: bool,
}

impl Guest {
    pub fn new(email: &str) -> Guest {
        Guest {
            email: email.trim().to_owned(),
            name: None,
            optional: false,
            response: "needsAction".into(),
            is_self: false,
            organizer: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderMethod {
    Popup,
    Email,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reminder {
    pub method: ReminderMethod,
    pub minutes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reminders {
    /// The calendar's default reminders.
    Default,
    Custom(Vec<Reminder>),
}

/// The event's Google Meet link, or what to do about one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Meet {
    None,
    /// The event has a conference; the text is its link.
    Existing(String),
    Add,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoDecline {
    None,
    /// Only invitations that arrive from now on.
    New,
    All,
}

impl AutoDecline {
    fn parse(text: Option<&str>) -> AutoDecline {
        match text {
            Some("declineAllConflictingInvitations") => AutoDecline::All,
            Some("declineOnlyNewConflictingInvitations") => AutoDecline::New,
            _ => AutoDecline::None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            AutoDecline::None => "declineNone",
            AutoDecline::New => "declineOnlyNewConflictingInvitations",
            AutoDecline::All => "declineAllConflictingInvitations",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Home,
    Office(String),
    Custom(String),
}

/// Google's `eventType` with the properties takt edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Event,
    FocusTime {
        decline: AutoDecline,
        message: String,
        do_not_disturb: bool,
    },
    OutOfOffice {
        decline: AutoDecline,
        message: String,
    },
    WorkingLocation(Place),
    /// A type takt has no fields for (`birthday`, `fromGmail`).
    Other(String),
}

impl Kind {
    pub fn focus_time() -> Kind {
        Kind::FocusTime {
            decline: AutoDecline::None,
            message: String::new(),
            do_not_disturb: true,
        }
    }

    pub fn out_of_office() -> Kind {
        Kind::OutOfOffice {
            decline: AutoDecline::All,
            message: String::new(),
        }
    }

    fn event_type(&self) -> Option<&str> {
        match self {
            Kind::Event => None,
            Kind::FocusTime { .. } => Some("focusTime"),
            Kind::OutOfOffice { .. } => Some("outOfOffice"),
            Kind::WorkingLocation(_) => Some("workingLocation"),
            Kind::Other(name) => Some(name),
        }
    }

    /// The key of this type's properties object and its content.
    fn properties(&self) -> Option<(&'static str, Value)> {
        let text = |s: &str| {
            if s.trim().is_empty() {
                Value::Null
            } else {
                s.trim().into()
            }
        };
        match self {
            Kind::Event | Kind::Other(_) => None,
            Kind::FocusTime {
                decline,
                message,
                do_not_disturb,
            } => Some((
                "focusTimeProperties",
                json!({
                    "autoDeclineMode": decline.as_str(),
                    "declineMessage": text(message),
                    "chatStatus": if *do_not_disturb { "doNotDisturb" } else { "available" },
                }),
            )),
            Kind::OutOfOffice { decline, message } => Some((
                "outOfOfficeProperties",
                json!({"autoDeclineMode": decline.as_str(), "declineMessage": text(message)}),
            )),
            Kind::WorkingLocation(place) => Some((
                "workingLocationProperties",
                match place {
                    Place::Home => json!({
                        "type": "homeOffice", "homeOffice": {},
                        "officeLocation": null, "customLocation": null,
                    }),
                    Place::Office(label) => json!({
                        "type": "officeLocation", "officeLocation": {"label": text(label)},
                        "homeOffice": null, "customLocation": null,
                    }),
                    Place::Custom(label) => json!({
                        "type": "customLocation", "customLocation": {"label": text(label)},
                        "homeOffice": null, "officeLocation": null,
                    }),
                },
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventDraft {
    pub summary: String,
    pub location: String,
    /// As Google stores it, which may be HTML.
    pub description: String,
    pub time: DraftTime,
    /// `None` uses the calendar's colour.
    pub color_id: Option<String>,
    pub guests: Vec<Guest>,
    pub reminders: Reminders,
    pub meet: Meet,
    pub kind: Kind,
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

fn parse_zone(value: &Value) -> Option<Tz> {
    value.as_str()?.parse().ok()
}

fn parse_instant(value: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.as_str()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

fn parse_day(value: &Value) -> Option<NaiveDate> {
    value.as_str()?.parse().ok()
}

fn rfc3339(local: NaiveDateTime, zone: Tz) -> String {
    local_instant(local, zone).to_rfc3339_opts(SecondsFormat::Secs, true)
}

impl EventDraft {
    /// A new, empty event at `time`.
    pub fn new(time: DraftTime) -> EventDraft {
        EventDraft {
            summary: String::new(),
            location: String::new(),
            description: String::new(),
            time,
            color_id: None,
            guests: Vec::new(),
            reminders: Reminders::Default,
            meet: Meet::None,
            kind: Kind::Event,
        }
    }

    /// Reads the editable fields of an event. `None` when it has no usable
    /// start and end (a cancelled stub). Timed events without a zone of
    /// their own are read in `display`.
    pub fn from_raw(raw: &Value, display: Tz) -> Option<EventDraft> {
        let (start, end) = (&raw["start"], &raw["end"]);
        let time = if let Some(first) = parse_day(&start["date"]) {
            let last = parse_day(&end["date"]).map_or(first, |end| end - TimeDelta::days(1));
            DraftTime::AllDay {
                first,
                last: last.max(first),
            }
        } else {
            let zone = parse_zone(&start["timeZone"]).unwrap_or(display);
            let from = parse_instant(&start["dateTime"])?;
            let to = parse_instant(&end["dateTime"]).unwrap_or(from);
            DraftTime::Timed {
                start: from.with_timezone(&zone).naive_local(),
                end: to.with_timezone(&zone).naive_local(),
                zone,
            }
        };
        let guests = raw["attendees"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter(|a| a["email"].is_string())
                    .map(|a| Guest {
                        email: text(&a["email"]),
                        name: a["displayName"].as_str().map(str::to_owned),
                        optional: a["optional"].as_bool() == Some(true),
                        response: a["responseStatus"]
                            .as_str()
                            .unwrap_or("needsAction")
                            .to_owned(),
                        is_self: a["self"].as_bool() == Some(true),
                        organizer: a["organizer"].as_bool() == Some(true),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let reminders = match (
            raw["reminders"]["useDefault"].as_bool(),
            raw["reminders"]["overrides"].as_array(),
        ) {
            (Some(false), overrides) => Reminders::Custom(
                overrides
                    .into_iter()
                    .flatten()
                    .filter_map(|r| {
                        Some(Reminder {
                            method: if r["method"] == "email" {
                                ReminderMethod::Email
                            } else {
                                ReminderMethod::Popup
                            },
                            minutes: u32::try_from(r["minutes"].as_u64()?).ok()?,
                        })
                    })
                    .collect(),
            ),
            _ => Reminders::Default,
        };
        let link = raw["hangoutLink"].as_str().or_else(|| {
            raw["conferenceData"]["entryPoints"]
                .as_array()?
                .iter()
                .find_map(|p| p["uri"].as_str())
        });
        let meet = match link {
            Some(link) => Meet::Existing(link.to_owned()),
            None if raw["conferenceData"].is_object() => Meet::Existing("Video call".into()),
            None => Meet::None,
        };
        let kind = match raw["eventType"].as_str() {
            None | Some("default") => Kind::Event,
            Some("focusTime") => {
                let p = &raw["focusTimeProperties"];
                Kind::FocusTime {
                    decline: AutoDecline::parse(p["autoDeclineMode"].as_str()),
                    message: text(&p["declineMessage"]),
                    do_not_disturb: p["chatStatus"] == "doNotDisturb",
                }
            }
            Some("outOfOffice") => {
                let p = &raw["outOfOfficeProperties"];
                Kind::OutOfOffice {
                    decline: AutoDecline::parse(p["autoDeclineMode"].as_str()),
                    message: text(&p["declineMessage"]),
                }
            }
            Some("workingLocation") => {
                let p = &raw["workingLocationProperties"];
                Kind::WorkingLocation(match p["type"].as_str() {
                    Some("officeLocation") => Place::Office(text(&p["officeLocation"]["label"])),
                    Some("customLocation") => Place::Custom(text(&p["customLocation"]["label"])),
                    _ => Place::Home,
                })
            }
            Some(other) => Kind::Other(other.to_owned()),
        };
        Some(EventDraft {
            summary: text(&raw["summary"]),
            location: text(&raw["location"]),
            description: text(&raw["description"]),
            time,
            color_id: raw["colorId"].as_str().map(str::to_owned),
            guests,
            reminders,
            meet,
            kind,
        })
    }

    /// Why the draft cannot be saved, in words for the user.
    pub fn problem(&self) -> Option<String> {
        match self.time {
            DraftTime::AllDay { first, last } if last < first => {
                return Some("The event ends before it starts.".into());
            }
            DraftTime::Timed { start, end, .. } if end < start => {
                return Some("The event ends before it starts.".into());
            }
            _ => {}
        }
        if self.time.is_all_day() && matches!(self.kind, Kind::FocusTime { .. }) {
            return Some("Focus time needs a start and an end time.".into());
        }
        if let Some(bad) = self.guests.iter().find(|g| !looks_like_email(&g.email)) {
            return Some(format!("\"{}\" is not an email address.", bad.email));
        }
        None
    }

    fn start_json(&self) -> Value {
        match self.time {
            DraftTime::AllDay { first, .. } => {
                json!({"date": first.to_string(), "dateTime": null, "timeZone": null})
            }
            DraftTime::Timed { start, zone, .. } => {
                json!({"dateTime": rfc3339(start, zone), "timeZone": zone.name(), "date": null})
            }
        }
    }

    /// `end_zone` is the zone to write the end in when it is not the start's.
    fn end_json(&self, end_zone: Option<Tz>) -> Value {
        match self.time {
            DraftTime::AllDay { last, .. } => json!({
                "date": (last + TimeDelta::days(1)).to_string(), "dateTime": null, "timeZone": null,
            }),
            DraftTime::Timed { end, zone, .. } => {
                let instant = local_instant(end, zone);
                let shown = end_zone.unwrap_or(zone);
                json!({
                    "dateTime": instant.with_timezone(&shown).to_rfc3339_opts(SecondsFormat::Secs, true),
                    "timeZone": shown.name(),
                    "date": null,
                })
            }
        }
    }

    /// The attendee list to send: each guest's own JSON from `original` with
    /// the edits on top, so fields takt does not show survive.
    fn attendees_json(&self, original: &Value) -> Value {
        let known = original["attendees"].as_array();
        let list: Vec<Value> = self
            .guests
            .iter()
            .map(|guest| {
                let before = known.and_then(|list| {
                    list.iter().find(|a| {
                        a["email"]
                            .as_str()
                            .is_some_and(|e| e.eq_ignore_ascii_case(&guest.email))
                    })
                });
                let mut entry = before
                    .cloned()
                    .unwrap_or_else(|| json!({"email": guest.email}));
                let was_optional = entry["optional"].as_bool() == Some(true);
                if guest.optional != was_optional {
                    entry["optional"] = guest.optional.into();
                }
                let was = entry["responseStatus"].as_str().unwrap_or("needsAction");
                if guest.response != was {
                    entry["responseStatus"] = guest.response.clone().into();
                }
                entry
            })
            .collect();
        Value::Array(list)
    }

    fn reminders_json(&self) -> Value {
        match &self.reminders {
            Reminders::Default => json!({"useDefault": true, "overrides": null}),
            Reminders::Custom(list) => json!({
                "useDefault": false,
                "overrides": list.iter().map(|r| json!({
                    "method": match r.method {
                        ReminderMethod::Popup => "popup",
                        ReminderMethod::Email => "email",
                    },
                    "minutes": r.minutes,
                })).collect::<Vec<_>>(),
            }),
        }
    }

    /// The fields to send to `events.patch`: only those that differ from
    /// `original`, the raw JSON this draft was read from. Empty when nothing
    /// changed. `request_id` names a new Meet link request, if one is made.
    pub fn patch(&self, original: &Value, display: Tz, request_id: &str) -> Value {
        let mut patch = Map::new();
        let Some(before) = EventDraft::from_raw(original, display) else {
            return Value::Object(patch);
        };
        let optional_text = |s: &str| {
            if s.is_empty() { Value::Null } else { s.into() }
        };
        if self.summary != before.summary {
            patch.insert("summary".into(), self.summary.clone().into());
        }
        if self.location != before.location {
            patch.insert("location".into(), optional_text(&self.location));
        }
        if self.description != before.description {
            patch.insert("description".into(), optional_text(&self.description));
        }
        if self.time != before.time {
            let (start, end) = self.time.instants(display);
            let (start_before, end_before) = before.time.instants(display);
            let reshaped = self.time.is_all_day() != before.time.is_all_day();
            let rezoned = matches!(
                (self.time, before.time),
                (DraftTime::Timed { zone: a, .. }, DraftTime::Timed { zone: b, .. }) if a != b
            );
            if reshaped || rezoned || start != start_before {
                patch.insert("start".into(), self.start_json());
            }
            if reshaped || rezoned || end != end_before {
                // An end kept in its own zone stays there unless the zone was edited.
                let end_zone = parse_zone(&original["end"]["timeZone"]).filter(|_| !rezoned);
                patch.insert("end".into(), self.end_json(end_zone));
            }
        }
        if self.color_id != before.color_id {
            patch.insert(
                "colorId".into(),
                self.color_id.clone().map_or(Value::Null, Value::from),
            );
        }
        if self.guests != before.guests {
            patch.insert("attendees".into(), self.attendees_json(original));
        }
        if self.reminders != before.reminders {
            patch.insert("reminders".into(), self.reminders_json());
        }
        match self.meet {
            Meet::Add if before.meet == Meet::None => {
                patch.insert("conferenceData".into(), meet_request(request_id));
            }
            Meet::Remove if before.meet != Meet::None => {
                patch.insert("conferenceData".into(), Value::Null);
            }
            _ => {}
        }
        // The type itself cannot change after creation; its properties can.
        if self.kind != before.kind
            && self.kind.event_type() == before.kind.event_type()
            && let Some((key, properties)) = self.kind.properties()
        {
            patch.insert(key.into(), properties);
        }
        Value::Object(patch)
    }

    /// The body for `events.insert`.
    pub fn insert_body(&self, request_id: &str) -> Value {
        let mut body = Map::new();
        let mut put = |key: &str, value: &str| {
            if !value.is_empty() {
                body.insert(key.into(), value.into());
            }
        };
        put("summary", &self.summary);
        put("location", &self.location);
        put("description", &self.description);
        if let Some(color) = &self.color_id {
            put("colorId", color);
        }
        body.insert("start".into(), without_nulls(self.start_json()));
        body.insert("end".into(), without_nulls(self.end_json(None)));
        if !self.guests.is_empty() {
            body.insert("attendees".into(), self.attendees_json(&Value::Null));
        }
        if let Reminders::Custom(_) = self.reminders {
            body.insert("reminders".into(), self.reminders_json());
        }
        if self.meet == Meet::Add {
            body.insert("conferenceData".into(), meet_request(request_id));
        }
        if let Some(event_type) = self.kind.event_type() {
            body.insert("eventType".into(), event_type.into());
        }
        if let Some((key, properties)) = self.kind.properties() {
            body.insert(key.into(), without_nulls(properties));
        }
        match self.kind {
            // Google requires these for the type.
            Kind::WorkingLocation(_) => {
                body.insert("transparency".into(), "transparent".into());
                body.insert("visibility".into(), "public".into());
            }
            Kind::FocusTime { .. } | Kind::OutOfOffice { .. } => {
                body.insert("transparency".into(), "opaque".into());
            }
            Kind::Event | Kind::Other(_) => {}
        }
        Value::Object(body)
    }
}

fn meet_request(request_id: &str) -> Value {
    json!({"createRequest": {
        "requestId": request_id,
        "conferenceSolutionKey": {"type": "hangoutsMeet"},
    }})
}

/// Drops `null` members, which mean "clear" in a patch and nothing in an insert.
fn without_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, without_nulls(v)))
                .collect(),
        ),
        other => other,
    }
}

pub fn looks_like_email(text: &str) -> bool {
    let text = text.trim();
    match text.split_once('@') {
        Some((name, domain)) => {
            !name.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !text.contains(char::is_whitespace)
        }
        None => false,
    }
}

/// The top-level fields of `patch` as they are in `original`, `null` where
/// absent: what sync compares with Google's copy after an etag mismatch.
pub fn base_of(original: &Value, patch: &Value) -> Value {
    let fields = patch
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, _)| {
            (
                key.clone(),
                original.get(key).cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    Value::Object(fields)
}

/// One occurrence of a series as if it were an event of its own: the series'
/// JSON with this occurrence's times and no rule. Edits are compared with it,
/// and the patch goes to the occurrence's instance ID.
pub fn occurrence_raw(series: &Value, instance_id: &str, start: When, end: When) -> Value {
    let mut raw = series.clone();
    let zone = parse_zone(&series["start"]["timeZone"]);
    let side = |when: When, zone_name: &Value| match when {
        When::Date(date) => json!({"date": date.to_string()}),
        When::At(instant) => {
            let mut side = match zone {
                Some(zone) => json!({
                    "dateTime": instant.with_timezone(&zone).to_rfc3339_opts(SecondsFormat::Secs, true),
                }),
                None => json!({"dateTime": instant.to_rfc3339_opts(SecondsFormat::Secs, true)}),
            };
            if zone_name.is_string() {
                side["timeZone"] = zone_name.clone();
            }
            side
        }
    };
    raw["start"] = side(start, &series["start"]["timeZone"]);
    raw["end"] = side(end, &series["end"]["timeZone"]);
    raw["id"] = instance_id.into();
    if let Some(fields) = raw.as_object_mut() {
        fields.remove("recurrence");
        fields.remove("etag");
    }
    raw
}

/// The parts of an event that differ between two versions of its JSON, in
/// words: what to tell the user changed in Google while they were editing.
pub fn changed_parts(old: &Value, new: &Value, display: Tz) -> Vec<&'static str> {
    let (Some(a), Some(b)) = (
        EventDraft::from_raw(old, display),
        EventDraft::from_raw(new, display),
    ) else {
        return Vec::new();
    };
    let mut parts = Vec::new();
    let mut note = |changed: bool, name: &'static str| {
        if changed {
            parts.push(name);
        }
    };
    note(a.summary != b.summary, "title");
    note(a.time != b.time, "time");
    note(a.location != b.location, "location");
    note(a.description != b.description, "description");
    note(a.color_id != b.color_id, "colour");
    note(a.guests != b.guests, "guests");
    note(a.reminders != b.reminders, "reminders");
    note(a.meet != b.meet, "video call");
    note(a.kind != b.kind, "event type");
    parts
}

/// Whether the signed-in account organizes the event and it has other guests:
/// the case where Google can email people about a change.
pub fn has_guests_to_notify(raw: &Value) -> bool {
    let others = raw["attendees"]
        .as_array()
        .is_some_and(|list| list.iter().any(|a| a["self"].as_bool() != Some(true)));
    others && raw["organizer"]["self"].as_bool() == Some(true)
}

#[cfg(test)]
mod tests {
    use chrono::NaiveTime;

    use super::*;

    const LONDON: Tz = chrono_tz::Europe::London;
    const NEW_YORK: Tz = chrono_tz::America::New_York;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap()
    }

    fn at(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
        d(day).and_time(NaiveTime::from_hms_opt(hour, minute, 0).unwrap())
    }

    fn lunch() -> Value {
        json!({
            "id": "e1", "etag": "\"1\"", "summary": "Lunch", "location": "Cafe",
            "start": {"dateTime": "2026-10-20T12:00:00+01:00", "timeZone": "Europe/London"},
            "end": {"dateTime": "2026-10-20T13:00:00+01:00", "timeZone": "Europe/London"},
            "organizer": {"email": "me@example.com", "self": true},
            "attendees": [
                {"email": "me@example.com", "self": true, "organizer": true, "responseStatus": "accepted"},
                {"email": "sam@example.com", "displayName": "Sam", "responseStatus": "needsAction", "comment": "maybe late"}
            ],
            "reminders": {"useDefault": true},
            "extendedProperties": {"private": {"k": "v"}}
        })
    }

    fn draft(raw: &Value) -> EventDraft {
        EventDraft::from_raw(raw, LONDON).unwrap()
    }

    fn patch(draft: &EventDraft, raw: &Value) -> Value {
        draft.patch(raw, LONDON, "req-1")
    }

    #[test]
    fn reads_the_editable_fields() {
        let draft = draft(&lunch());
        assert_eq!(draft.summary, "Lunch");
        assert_eq!(draft.location, "Cafe");
        assert_eq!(
            draft.time,
            DraftTime::Timed {
                start: at(20, 12, 0),
                end: at(20, 13, 0),
                zone: LONDON
            }
        );
        assert_eq!(draft.guests.len(), 2);
        assert!(draft.guests[0].is_self && draft.guests[0].organizer);
        assert_eq!(draft.guests[1].name.as_deref(), Some("Sam"));
        assert_eq!(draft.reminders, Reminders::Default);
        assert_eq!(draft.meet, Meet::None);
        assert_eq!(draft.kind, Kind::Event);
    }

    #[test]
    fn an_unchanged_draft_patches_nothing() {
        let raw = lunch();
        assert_eq!(patch(&draft(&raw), &raw), json!({}));
    }

    #[test]
    fn only_changed_text_fields_are_sent_and_cleared_ones_are_null() {
        let raw = lunch();
        let mut draft = draft(&raw);
        draft.summary = "Late lunch".into();
        draft.location.clear();
        draft.description = "Bring cake".into();
        assert_eq!(
            patch(&draft, &raw),
            json!({"summary": "Late lunch", "location": null, "description": "Bring cake"})
        );
    }

    #[test]
    fn a_resize_sends_only_the_end() {
        let raw = lunch();
        let mut draft = draft(&raw);
        draft.time = DraftTime::Timed {
            start: at(20, 12, 0),
            end: at(20, 13, 30),
            zone: LONDON,
        };
        assert_eq!(
            patch(&draft, &raw),
            json!({"end": {
                "dateTime": "2026-10-20T13:30:00+01:00", "timeZone": "Europe/London", "date": null
            }})
        );
    }

    #[test]
    fn a_move_sends_start_and_end_in_the_events_own_zone() {
        let mut raw = lunch();
        raw["start"] =
            json!({"dateTime": "2026-10-20T07:00:00-04:00", "timeZone": "America/New_York"});
        raw["end"] =
            json!({"dateTime": "2026-10-20T08:00:00-04:00", "timeZone": "America/New_York"});
        let mut draft = draft(&raw);
        assert_eq!(
            draft.time,
            DraftTime::Timed {
                start: at(20, 7, 0),
                end: at(20, 8, 0),
                zone: NEW_YORK
            }
        );
        // Dragged to 15:00–16:00 London time.
        let start = local_instant(at(20, 15, 0), LONDON).with_timezone(&Utc);
        draft.time = draft.time.at(start, start + TimeDelta::hours(1), LONDON);
        assert_eq!(
            patch(&draft, &raw),
            json!({
                "start": {"dateTime": "2026-10-20T10:00:00-04:00", "timeZone": "America/New_York", "date": null},
                "end": {"dateTime": "2026-10-20T11:00:00-04:00", "timeZone": "America/New_York", "date": null},
            })
        );
    }

    #[test]
    fn an_end_in_another_zone_keeps_that_zone() {
        // A flight: leaves London 10:00, lands New York 13:00 local.
        let mut raw = lunch();
        raw["start"] =
            json!({"dateTime": "2026-10-20T10:00:00+01:00", "timeZone": "Europe/London"});
        raw["end"] =
            json!({"dateTime": "2026-10-20T13:00:00-04:00", "timeZone": "America/New_York"});
        let mut draft = draft(&raw);
        let DraftTime::Timed { start, end, zone } = draft.time else {
            panic!("timed")
        };
        assert_eq!(end, at(20, 18, 0), "the end is shown in the start's zone");
        draft.time = DraftTime::Timed {
            start,
            end: end + TimeDelta::minutes(30),
            zone,
        };
        assert_eq!(
            patch(&draft, &raw),
            json!({"end": {
                "dateTime": "2026-10-20T13:30:00-04:00", "timeZone": "America/New_York", "date": null
            }})
        );
    }

    #[test]
    fn changing_the_zone_rewrites_both_sides_in_it() {
        let raw = lunch();
        let mut draft = draft(&raw);
        draft.time = DraftTime::Timed {
            start: at(20, 12, 0),
            end: at(20, 13, 0),
            zone: NEW_YORK,
        };
        assert_eq!(
            patch(&draft, &raw),
            json!({
                "start": {"dateTime": "2026-10-20T12:00:00-04:00", "timeZone": "America/New_York", "date": null},
                "end": {"dateTime": "2026-10-20T13:00:00-04:00", "timeZone": "America/New_York", "date": null},
            })
        );
    }

    #[test]
    fn making_an_event_all_day_clears_its_times() {
        let raw = lunch();
        let mut draft = draft(&raw);
        draft.time = DraftTime::AllDay {
            first: d(20),
            last: d(21),
        };
        assert_eq!(
            patch(&draft, &raw),
            json!({
                "start": {"date": "2026-10-20", "dateTime": null, "timeZone": null},
                "end": {"date": "2026-10-22", "dateTime": null, "timeZone": null},
            })
        );
    }

    #[test]
    fn all_day_events_read_their_last_day() {
        let raw = json!({"start": {"date": "2026-10-20"}, "end": {"date": "2026-10-23"}});
        let read = draft(&raw);
        assert_eq!(
            read.time,
            DraftTime::AllDay {
                first: d(20),
                last: d(22)
            }
        );
        assert_eq!(patch(&read, &raw), json!({}));
        assert!(EventDraft::from_raw(&json!({"id": "x", "status": "cancelled"}), LONDON).is_none());
    }

    #[test]
    fn guests_keep_their_own_json_and_a_reply_changes_only_its_status() {
        let raw = lunch();
        let mut draft = draft(&raw);
        draft.guests[0].response = "declined".into();
        draft.guests[1].optional = true;
        draft.guests.push(Guest::new(" kim@example.com "));
        assert_eq!(
            patch(&draft, &raw),
            json!({"attendees": [
                {"email": "me@example.com", "self": true, "organizer": true, "responseStatus": "declined"},
                {"email": "sam@example.com", "displayName": "Sam", "responseStatus": "needsAction",
                 "comment": "maybe late", "optional": true},
                {"email": "kim@example.com"}
            ]})
        );

        draft.guests.clear();
        assert_eq!(patch(&draft, &raw), json!({"attendees": []}));
    }

    #[test]
    fn reminders_switch_between_default_and_custom() {
        let raw = lunch();
        let mut edited = draft(&raw);
        edited.reminders = Reminders::Custom(vec![
            Reminder {
                method: ReminderMethod::Popup,
                minutes: 10,
            },
            Reminder {
                method: ReminderMethod::Email,
                minutes: 1440,
            },
        ]);
        let custom = json!({"useDefault": false, "overrides": [
            {"method": "popup", "minutes": 10}, {"method": "email", "minutes": 1440}
        ]});
        assert_eq!(patch(&edited, &raw), json!({"reminders": custom}));

        let mut raw = raw;
        raw["reminders"] = custom;
        let mut back = draft(&raw);
        assert_eq!(back.reminders, edited.reminders);
        back.reminders = Reminders::Default;
        assert_eq!(
            patch(&back, &raw),
            json!({"reminders": {"useDefault": true, "overrides": null}})
        );
    }

    #[test]
    fn meet_is_requested_or_removed() {
        let raw = lunch();
        let mut edited = draft(&raw);
        edited.meet = Meet::Add;
        assert_eq!(
            patch(&edited, &raw),
            json!({"conferenceData": {"createRequest": {
                "requestId": "req-1", "conferenceSolutionKey": {"type": "hangoutsMeet"}
            }}})
        );

        let mut raw = raw;
        raw["hangoutLink"] = "https://meet.google.com/abc-defg-hij".into();
        raw["conferenceData"] = json!({"conferenceId": "abc-defg-hij"});
        let mut edited = draft(&raw);
        assert_eq!(
            edited.meet,
            Meet::Existing("https://meet.google.com/abc-defg-hij".into())
        );
        assert_eq!(patch(&edited, &raw), json!({}));
        edited.meet = Meet::Remove;
        assert_eq!(patch(&edited, &raw), json!({"conferenceData": null}));
    }

    #[test]
    fn colour_is_set_or_cleared() {
        let raw = lunch();
        let mut edited = draft(&raw);
        edited.color_id = Some("11".into());
        assert_eq!(patch(&edited, &raw), json!({"colorId": "11"}));
        let mut raw = raw;
        raw["colorId"] = "11".into();
        let mut edited = draft(&raw);
        edited.color_id = None;
        assert_eq!(patch(&edited, &raw), json!({"colorId": null}));
    }

    #[test]
    fn an_event_types_properties_are_patched_but_not_its_type() {
        let mut raw = lunch();
        raw["eventType"] = "focusTime".into();
        raw["focusTimeProperties"] =
            json!({"autoDeclineMode": "declineNone", "chatStatus": "doNotDisturb"});
        let mut edited = draft(&raw);
        assert_eq!(edited.kind, Kind::focus_time());
        edited.kind = Kind::FocusTime {
            decline: AutoDecline::All,
            message: "Heads down".into(),
            do_not_disturb: false,
        };
        assert_eq!(
            patch(&edited, &raw),
            json!({"focusTimeProperties": {
                "autoDeclineMode": "declineAllConflictingInvitations",
                "declineMessage": "Heads down",
                "chatStatus": "available",
            }})
        );
        edited.kind = Kind::out_of_office();
        assert_eq!(patch(&edited, &raw), json!({}), "the type cannot change");
    }

    #[test]
    fn a_new_event_sends_only_what_it_has() {
        let mut new = EventDraft::new(DraftTime::Timed {
            start: at(20, 9, 0),
            end: at(20, 10, 0),
            zone: LONDON,
        });
        new.summary = "Standup".into();
        assert_eq!(
            new.insert_body("req-1"),
            json!({
                "summary": "Standup",
                "start": {"dateTime": "2026-10-20T09:00:00+01:00", "timeZone": "Europe/London"},
                "end": {"dateTime": "2026-10-20T10:00:00+01:00", "timeZone": "Europe/London"},
            })
        );

        let mut day = EventDraft::new(DraftTime::AllDay {
            first: d(20),
            last: d(20),
        });
        day.guests.push(Guest::new("sam@example.com"));
        day.meet = Meet::Add;
        day.color_id = Some("5".into());
        assert_eq!(
            day.insert_body("req-2"),
            json!({
                "colorId": "5",
                "start": {"date": "2026-10-20"},
                "end": {"date": "2026-10-21"},
                "attendees": [{"email": "sam@example.com"}],
                "conferenceData": {"createRequest": {
                    "requestId": "req-2", "conferenceSolutionKey": {"type": "hangoutsMeet"}
                }},
            })
        );
    }

    #[test]
    fn special_event_types_carry_what_google_requires() {
        let time = DraftTime::Timed {
            start: at(20, 9, 0),
            end: at(20, 11, 0),
            zone: LONDON,
        };
        let mut focus = EventDraft::new(time);
        focus.kind = Kind::focus_time();
        let body = focus.insert_body("r");
        assert_eq!(body["eventType"], "focusTime");
        assert_eq!(body["transparency"], "opaque");
        assert_eq!(
            body["focusTimeProperties"],
            json!({"autoDeclineMode": "declineNone", "chatStatus": "doNotDisturb"})
        );

        let mut away = EventDraft::new(time);
        away.kind = Kind::OutOfOffice {
            decline: AutoDecline::New,
            message: "Back Monday".into(),
        };
        assert_eq!(
            away.insert_body("r")["outOfOfficeProperties"],
            json!({
                "autoDeclineMode": "declineOnlyNewConflictingInvitations",
                "declineMessage": "Back Monday",
            })
        );

        let mut place = EventDraft::new(DraftTime::AllDay {
            first: d(20),
            last: d(20),
        });
        place.kind = Kind::WorkingLocation(Place::Office("HQ".into()));
        let body = place.insert_body("r");
        assert_eq!(body["eventType"], "workingLocation");
        assert_eq!(body["visibility"], "public");
        assert_eq!(body["transparency"], "transparent");
        assert_eq!(
            body["workingLocationProperties"],
            json!({"type": "officeLocation", "officeLocation": {"label": "HQ"}})
        );
        let read = draft(&body);
        assert_eq!(read.kind, Kind::WorkingLocation(Place::Office("HQ".into())));
    }

    #[test]
    fn problems_are_named() {
        let mut draft = draft(&lunch());
        assert_eq!(draft.problem(), None);
        draft.guests.push(Guest::new("not an address"));
        assert_eq!(
            draft.problem().as_deref(),
            Some("\"not an address\" is not an email address.")
        );
        draft.guests.pop();
        draft.time = DraftTime::Timed {
            start: at(20, 12, 0),
            end: at(20, 11, 0),
            zone: LONDON,
        };
        assert_eq!(
            draft.problem().as_deref(),
            Some("The event ends before it starts.")
        );
        draft.time = DraftTime::AllDay {
            first: d(20),
            last: d(20),
        };
        draft.kind = Kind::focus_time();
        assert!(draft.problem().unwrap().contains("Focus time"));
    }

    #[test]
    fn the_base_holds_the_patched_fields_as_they_were() {
        let raw = lunch();
        let patch = json!({"summary": "x", "description": "y", "start": {}});
        assert_eq!(
            base_of(&raw, &patch),
            json!({"summary": "Lunch", "description": null, "start": raw["start"]})
        );
    }

    #[test]
    fn an_occurrence_reads_as_its_own_event() {
        let mut series = lunch();
        series["recurrence"] = json!(["RRULE:FREQ=WEEKLY"]);
        let start = local_instant(at(27, 12, 0), LONDON).with_timezone(&Utc);
        let raw = occurrence_raw(
            &series,
            "e1_20261027T120000Z",
            When::At(start),
            When::At(start + TimeDelta::hours(1)),
        );
        assert_eq!(raw["id"], "e1_20261027T120000Z");
        assert!(raw.get("recurrence").is_none() && raw.get("etag").is_none());
        // 27 October is after the clocks go back: GMT, not BST.
        assert_eq!(
            raw["start"],
            json!({"dateTime": "2026-10-27T12:00:00Z", "timeZone": "Europe/London"})
        );
        let mut edited = draft(&raw);
        assert_eq!(patch(&edited, &raw), json!({}));
        edited.summary = "Lunch (moved)".into();
        assert_eq!(patch(&edited, &raw), json!({"summary": "Lunch (moved)"}));

        let day = json!({"start": {"date": "2026-10-06"}, "end": {"date": "2026-10-07"}});
        let raw = occurrence_raw(&day, "b_20261013", When::Date(d(13)), When::Date(d(14)));
        assert_eq!(raw["start"], json!({"date": "2026-10-13"}));
        assert_eq!(raw["end"], json!({"date": "2026-10-14"}));
    }

    #[test]
    fn changed_parts_are_listed_in_words() {
        let old = lunch();
        let mut new = lunch();
        new["summary"] = "Brunch".into();
        new["start"]["dateTime"] = "2026-10-20T11:00:00+01:00".into();
        new["extendedProperties"] = json!({});
        assert_eq!(changed_parts(&old, &new, LONDON), ["title", "time"]);
        assert!(changed_parts(&old, &old, LONDON).is_empty());
    }

    #[test]
    fn only_an_organizer_with_other_guests_can_notify() {
        assert!(has_guests_to_notify(&lunch()));
        let mut invited = lunch();
        invited["organizer"] = json!({"email": "sam@example.com"});
        assert!(!has_guests_to_notify(&invited));
        let mut alone = lunch();
        alone["attendees"] = json!([{"email": "me@example.com", "self": true}]);
        assert!(!has_guests_to_notify(&alone));
        assert!(!has_guests_to_notify(&json!({"organizer": {"self": true}})));
    }

    #[test]
    fn wall_clock_times_that_do_not_exist_or_happen_twice() {
        // 29 March 2026, 01:30 does not exist in London; 25 October, 01:30 happens twice.
        let march = NaiveDate::from_ymd_opt(2026, 3, 29)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        assert_eq!(
            local_instant(march, LONDON).to_rfc3339(),
            "2026-03-29T02:30:00+01:00"
        );
        assert_eq!(
            local_instant(at(25, 1, 30), LONDON).to_rfc3339(),
            "2026-10-25T01:30:00+01:00"
        );
    }

    #[test]
    fn email_addresses_are_checked_loosely() {
        for good in ["a@b.co", " sam@example.com "] {
            assert!(looks_like_email(good), "{good}");
        }
        for bad in [
            "",
            "sam",
            "sam@",
            "@example.com",
            "sam@example",
            "a b@c.de",
            "a@.com",
        ] {
            assert!(!looks_like_email(bad), "{bad}");
        }
    }
}
