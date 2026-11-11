//! The parts of Google's resources takt reads. Every resource also keeps its
//! complete JSON in [`Item::raw`], so fields takt does not know survive.

use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::GoogleError;

#[derive(Debug, Clone, PartialEq)]
pub struct Item<T> {
    pub fields: T,
    pub raw: Value,
}

impl<T: DeserializeOwned> Item<T> {
    pub fn from_value(raw: Value) -> Result<Self, GoogleError> {
        let fields = T::deserialize(&raw).map_err(|e| {
            let id = raw.get("id").and_then(Value::as_str);
            match id {
                Some(id) => GoogleError::Decode(format!("item {id}: {e}")),
                None => GoogleError::Decode(e.to_string()),
            }
        })?;
        Ok(Self { fields, raw })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Page<T> {
    pub items: Vec<Item<T>>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
}

impl<T: DeserializeOwned> Page<T> {
    pub fn from_value(mut body: Value) -> Result<Self, GoogleError> {
        let items = match body.get_mut("items").map(Value::take) {
            Some(Value::Array(items)) => items
                .into_iter()
                .map(Item::from_value)
                .collect::<Result<Vec<_>, _>>()?,
            Some(Value::Null) | None => Vec::new(),
            Some(other) => {
                return Err(GoogleError::Decode(format!(
                    "items is not an array: {other}"
                )));
            }
        };
        Ok(Self {
            items,
            next_page_token: string_field(&body, "nextPageToken"),
            next_sync_token: string_field(&body, "nextSyncToken"),
        })
    }
}

fn string_field(body: &Value, key: &str) -> Option<String> {
    body.get(key).and_then(Value::as_str).map(str::to_owned)
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDateTime {
    pub date: Option<NaiveDate>,
    pub date_time: Option<DateTime<FixedOffset>>,
    pub time_zone: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attendee {
    pub email: Option<String>,
    #[serde(rename = "self", default)]
    pub is_self: bool,
    pub response_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventFields {
    pub id: String,
    pub status: Option<String>,
    pub etag: Option<String>,
    #[serde(rename = "iCalUID")]
    pub ical_uid: Option<String>,
    pub summary: Option<String>,
    pub location: Option<String>,
    pub color_id: Option<String>,
    pub event_type: Option<String>,
    pub start: Option<EventDateTime>,
    pub end: Option<EventDateTime>,
    #[serde(default)]
    pub recurrence: Vec<String>,
    pub recurring_event_id: Option<String>,
    pub original_start_time: Option<EventDateTime>,
    pub updated: Option<DateTime<Utc>>,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
}

impl EventFields {
    pub fn is_cancelled(&self) -> bool {
        self.status.as_deref() == Some("cancelled")
    }

    /// The signed-in user's RSVP, when they are an attendee.
    pub fn self_response(&self) -> Option<&str> {
        self.attendees
            .iter()
            .find(|a| a.is_self)
            .and_then(|a| a.response_status.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarListEntry {
    pub id: String,
    pub summary: Option<String>,
    pub summary_override: Option<String>,
    pub time_zone: Option<String>,
    pub color_id: Option<String>,
    pub background_color: Option<String>,
    pub foreground_color: Option<String>,
    pub access_role: Option<String>,
    #[serde(default)]
    pub selected: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub deleted: bool,
}

impl CalendarListEntry {
    pub fn display_name(&self) -> String {
        self.summary_override
            .clone()
            .or_else(|| self.summary.clone())
            .unwrap_or_else(|| self.id.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskListFields {
    pub id: String,
    pub title: Option<String>,
    pub updated: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFields {
    pub id: String,
    pub title: Option<String>,
    pub status: Option<String>,
    pub due: Option<DateTime<FixedOffset>>,
    pub parent: Option<String>,
    pub position: Option<String>,
    pub completed: Option<DateTime<FixedOffset>>,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub hidden: bool,
    pub updated: Option<DateTime<Utc>>,
}

impl TaskFields {
    /// The Tasks API keeps only the date of `due`; the time part is always midnight UTC.
    pub fn due_date(&self) -> Option<NaiveDate> {
        self.due.map(|d| d.date_naive())
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ColorDef {
    pub background: String,
    pub foreground: String,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct Colors {
    #[serde(default)]
    pub calendar: BTreeMap<String, ColorDef>,
    #[serde(default)]
    pub event: BTreeMap<String, ColorDef>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Setting {
    pub id: String,
    pub value: String,
}
