//! The Calendar and Tasks endpoints takt calls.

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::{Method, Url};
use serde_json::Value;

use crate::model::{
    CalendarListEntry, Colors, EventFields, Item, Page, Setting, TaskFields, TaskListFields,
};
use crate::{GoogleClient, GoogleError};

impl GoogleClient {
    pub async fn calendar_list_page(
        &self,
        sync_token: Option<&str>,
        page_token: Option<&str>,
    ) -> Result<Page<CalendarListEntry>, GoogleError> {
        let mut url = join(&self.endpoints.calendar, &["users", "me", "calendarList"])?;
        url.query_pairs_mut()
            .append_pair("maxResults", "250")
            .append_pair("showDeleted", "true")
            .append_pair("showHidden", "true");
        push_tokens(&mut url, sync_token, page_token);
        Page::from_value(self.get_json(url).await?)
    }

    pub async fn events_page(
        &self,
        calendar_id: &str,
        sync_token: Option<&str>,
        page_token: Option<&str>,
    ) -> Result<Page<EventFields>, GoogleError> {
        let mut url = join(
            &self.endpoints.calendar,
            &["calendars", calendar_id, "events"],
        )?;
        url.query_pairs_mut()
            .append_pair("maxResults", "2500")
            .append_pair("showDeleted", "true");
        push_tokens(&mut url, sync_token, page_token);
        Page::from_value(self.get_json(url).await?)
    }

    pub async fn colors(&self) -> Result<Item<Colors>, GoogleError> {
        let url = join(&self.endpoints.calendar, &["colors"])?;
        Item::from_value(self.get_json(url).await?)
    }

    pub async fn settings(&self) -> Result<Vec<Item<Setting>>, GoogleError> {
        let mut settings = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = join(&self.endpoints.calendar, &["users", "me", "settings"])?;
            url.query_pairs_mut().append_pair("maxResults", "250");
            push_tokens(&mut url, None, page_token.as_deref());
            let page: Page<Setting> = Page::from_value(self.get_json(url).await?)?;
            settings.extend(page.items);
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => return Ok(settings),
            }
        }
    }

    pub async fn task_lists(&self) -> Result<Vec<Item<TaskListFields>>, GoogleError> {
        let mut lists = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = join(&self.endpoints.tasks, &["users", "@me", "lists"])?;
            url.query_pairs_mut().append_pair("maxResults", "100");
            push_tokens(&mut url, None, page_token.as_deref());
            let page: Page<TaskListFields> = Page::from_value(self.get_json(url).await?)?;
            lists.extend(page.items);
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => return Ok(lists),
            }
        }
    }

    pub async fn tasks_page(
        &self,
        list_id: &str,
        updated_min: Option<DateTime<Utc>>,
        page_token: Option<&str>,
    ) -> Result<Page<TaskFields>, GoogleError> {
        let mut url = join(&self.endpoints.tasks, &["lists", list_id, "tasks"])?;
        url.query_pairs_mut()
            .append_pair("maxResults", "100")
            .append_pair("showCompleted", "true")
            .append_pair("showDeleted", "true")
            .append_pair("showHidden", "true");
        if let Some(since) = updated_min {
            url.query_pairs_mut().append_pair(
                "updatedMin",
                &since.to_rfc3339_opts(SecondsFormat::Secs, true),
            );
        }
        push_tokens(&mut url, None, page_token);
        Page::from_value(self.get_json(url).await?)
    }

    /// Google's own expansion of a series. Used only to capture test fixtures.
    pub async fn instances(
        &self,
        calendar_id: &str,
        event_id: &str,
        time_min: DateTime<Utc>,
        time_max: DateTime<Utc>,
    ) -> Result<Vec<Item<EventFields>>, GoogleError> {
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = join(
                &self.endpoints.calendar,
                &["calendars", calendar_id, "events", event_id, "instances"],
            )?;
            url.query_pairs_mut()
                .append_pair(
                    "timeMin",
                    &time_min.to_rfc3339_opts(SecondsFormat::Secs, true),
                )
                .append_pair(
                    "timeMax",
                    &time_max.to_rfc3339_opts(SecondsFormat::Secs, true),
                )
                .append_pair("maxResults", "2500");
            push_tokens(&mut url, None, page_token.as_deref());
            let page: Page<EventFields> = Page::from_value(self.get_json(url).await?)?;
            out.extend(page.items);
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => return Ok(out),
            }
        }
    }
}

/// Whether Google emails the guests about a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SendUpdates {
    #[default]
    None,
    All,
}

impl SendUpdates {
    fn as_str(self) -> &'static str {
        match self {
            SendUpdates::None => "none",
            SendUpdates::All => "all",
        }
    }
}

/// A new event ID Google accepts: 26 base32hex characters. Choosing the ID
/// before `events.insert` lets a repeated insert be told from a second event.
pub fn new_event_id() -> String {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuv";
    (0..26)
        .map(|_| ALPHABET[rand::random_range(0..ALPHABET.len())] as char)
        .collect()
}

/// The write endpoints. Every one takes and returns whole JSON, so fields
/// takt does not know pass through untouched.
impl GoogleClient {
    fn event_url(
        &self,
        calendar_id: &str,
        tail: &[&str],
        send_updates: Option<SendUpdates>,
    ) -> Result<Url, GoogleError> {
        let mut segments = vec!["calendars", calendar_id, "events"];
        segments.extend_from_slice(tail);
        let mut url = join(&self.endpoints.calendar, &segments)?;
        if let Some(send_updates) = send_updates {
            // conferenceDataVersion=1 lets a write add or remove a Meet link
            // and keeps existing conference data otherwise; supportsAttachments
            // says attachments in a body are meant, not forgotten.
            url.query_pairs_mut()
                .append_pair("sendUpdates", send_updates.as_str())
                .append_pair("conferenceDataVersion", "1")
                .append_pair("supportsAttachments", "true");
        }
        Ok(url)
    }

    pub async fn get_event(
        &self,
        calendar_id: &str,
        event_id: &str,
    ) -> Result<Item<EventFields>, GoogleError> {
        let url = self.event_url(calendar_id, &[event_id], None)?;
        Item::from_value(self.get_json(url).await?)
    }

    /// `events.insert`. With an `id` in `body`, inserting the same body twice
    /// fails the second time with [`GoogleError::Conflict`].
    pub async fn insert_event(
        &self,
        calendar_id: &str,
        body: &Value,
        send_updates: SendUpdates,
    ) -> Result<Item<EventFields>, GoogleError> {
        let url = self.event_url(calendar_id, &[], Some(send_updates))?;
        event_from(self.send_json(Method::POST, url, Some(body), None).await?)
    }

    /// `events.patch`: only the fields in `patch` change. A `null` clears a
    /// field. With `if_match`, fails with [`GoogleError::PreconditionFailed`]
    /// when the event changed since that etag.
    pub async fn patch_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        patch: &Value,
        if_match: Option<&str>,
        send_updates: SendUpdates,
    ) -> Result<Item<EventFields>, GoogleError> {
        let url = self.event_url(calendar_id, &[event_id], Some(send_updates))?;
        event_from(
            self.send_json(Method::PATCH, url, Some(patch), if_match)
                .await?,
        )
    }

    /// `events.delete`. Deleting an event that is already gone fails with
    /// [`GoogleError::Gone`] or [`GoogleError::NotFound`].
    pub async fn delete_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        if_match: Option<&str>,
        send_updates: SendUpdates,
    ) -> Result<(), GoogleError> {
        let mut url = join(
            &self.endpoints.calendar,
            &["calendars", calendar_id, "events", event_id],
        )?;
        url.query_pairs_mut()
            .append_pair("sendUpdates", send_updates.as_str());
        self.send_json(Method::DELETE, url, None, if_match).await?;
        Ok(())
    }

    /// `events.move`: changes which calendar of the same account organizes
    /// the event. Returns the event as it is in `destination`.
    pub async fn move_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        destination: &str,
        send_updates: SendUpdates,
    ) -> Result<Item<EventFields>, GoogleError> {
        let mut url = join(
            &self.endpoints.calendar,
            &["calendars", calendar_id, "events", event_id, "move"],
        )?;
        url.query_pairs_mut()
            .append_pair("destination", destination)
            .append_pair("sendUpdates", send_updates.as_str());
        event_from(self.send_json(Method::POST, url, None, None).await?)
    }
}

fn event_from(body: Option<Value>) -> Result<Item<EventFields>, GoogleError> {
    Item::from_value(body.ok_or_else(|| GoogleError::Decode("empty response body".into()))?)
}

/// Appends path segments to a base ending in `/`. Segments are percent-encoded,
/// so calendar IDs such as `en.usa#holiday@group.v.calendar.google.com` are safe.
pub(crate) fn join(base: &Url, segments: &[&str]) -> Result<Url, GoogleError> {
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| GoogleError::Decode(format!("base URL cannot take a path: {base}")))?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

fn push_tokens(url: &mut Url, sync_token: Option<&str>, page_token: Option<&str>) {
    let mut q = url.query_pairs_mut();
    if let Some(token) = sync_token {
        q.append_pair("syncToken", token);
    }
    if let Some(token) = page_token {
        q.append_pair("pageToken", token);
    }
}
