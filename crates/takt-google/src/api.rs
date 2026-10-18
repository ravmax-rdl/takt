//! The read endpoints sub-project 1 uses.

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::Url;

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

    pub async fn colors(&self) -> Result<Colors, GoogleError> {
        let url = join(&self.endpoints.calendar, &["colors"])?;
        serde_json::from_value(self.get_json(url).await?)
            .map_err(|e| GoogleError::Decode(e.to_string()))
    }

    pub async fn settings(&self) -> Result<Vec<Setting>, GoogleError> {
        let mut settings = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = join(&self.endpoints.calendar, &["users", "me", "settings"])?;
            url.query_pairs_mut().append_pair("maxResults", "250");
            push_tokens(&mut url, None, page_token.as_deref());
            let page: Page<Setting> = Page::from_value(self.get_json(url).await?)?;
            settings.extend(page.items.into_iter().map(|item| item.fields));
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
