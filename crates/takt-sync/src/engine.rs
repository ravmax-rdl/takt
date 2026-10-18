//! One sync round for one account, and what to do when Google says no.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use futures::stream::{self, StreamExt};
use takt_google::model::{Colors, Item, Page, Setting};
use takt_google::{GoogleClient, GoogleError, Url};
use takt_store::{ChangeSet, Store, StoreError};

use crate::SyncEvent;

pub const PARALLEL_CALENDARS: usize = 4;

/// The Tasks API has no sync tokens; ask again for a minute before the
/// checkpoint so clock skew cannot lose an update. Upserts make repeats harmless.
const TASKS_OVERLAP: TimeDelta = TimeDelta::seconds(60);

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Google(#[from] GoogleError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl SyncError {
    pub fn is_offline(&self) -> bool {
        matches!(self, SyncError::Google(GoogleError::Network(_)))
    }

    pub fn needs_reauth(&self) -> bool {
        matches!(self, SyncError::Google(GoogleError::InvalidGrant))
    }

    /// Lower is more serious: re-auth stops the account, offline changes its status.
    fn severity(&self) -> u8 {
        if self.needs_reauth() {
            0
        } else if self.is_offline() {
            1
        } else {
            2
        }
    }
}

fn calendar_url(client: &GoogleClient, segments: &[&str]) -> Result<Url, GoogleError> {
    let base = &client.endpoints().calendar;
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| GoogleError::Decode(format!("base URL cannot take a path: {base}")))?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

/// The error a round reports: the most serious, the earliest among equals.
fn most_serious(errors: Vec<SyncError>) -> Option<SyncError> {
    errors.into_iter().min_by_key(SyncError::severity)
}

/// 403 reasons that mean "not now" rather than "no access".
const TEMPORARY_FORBIDDEN: &[&str] = &["dailyLimitExceeded", "quotaExceeded"];

fn is_lost_access(error: &GoogleError) -> bool {
    match error {
        GoogleError::NotFound => true,
        GoogleError::Forbidden { reason } => !TEMPORARY_FORBIDDEN.contains(&reason.as_str()),
        _ => false,
    }
}

pub struct AccountEngine {
    account_id: String,
    client: GoogleClient,
    store: Arc<Mutex<Store>>,
    events: async_channel::Sender<SyncEvent>,
    meta_synced_at: Mutex<Option<Instant>>,
}

impl AccountEngine {
    pub fn new(
        account_id: impl Into<String>,
        client: GoogleClient,
        store: Arc<Mutex<Store>>,
        events: async_channel::Sender<SyncEvent>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            client,
            store,
            events,
            meta_synced_at: Mutex::new(None),
        }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// Calendar list, then settings and colours when asked, then every
    /// non-hidden calendar's events (primary first, four at a time), then tasks.
    ///
    /// One failing part does not stop the others. A revoked grant or lost
    /// connection on the calendar list or meta ends the round at once, since
    /// every later request would fail the same way. Otherwise the round runs to
    /// the end and returns the most serious error: re-auth, then offline, then
    /// the first other one.
    pub async fn round(&self, with_meta: bool) -> Result<(), SyncError> {
        let mut errors = Vec::new();
        if let Err(error) = self.sync_calendar_list().await {
            if error.needs_reauth() || error.is_offline() {
                return Err(error);
            }
            tracing::warn!(account = %self.account_id, %error, "calendar list sync failed; using stored calendars");
            errors.push(error);
        }
        if with_meta {
            match self.sync_meta().await {
                Ok(()) => {
                    *self.meta_synced_at.lock().expect("meta mutex poisoned") = Some(Instant::now())
                }
                Err(error) if error.needs_reauth() || error.is_offline() => return Err(error),
                Err(error) => {
                    tracing::warn!(account = %self.account_id, %error, "settings and colours sync failed");
                    errors.push(error);
                }
            }
        }
        let calendars: Vec<String> = self
            .with_store(|s| s.calendars(Some(&self.account_id)))?
            .into_iter()
            .filter(|c| !c.hidden)
            .map(|c| c.calendar_id)
            .collect();
        let pending: Vec<futures::future::BoxFuture<'_, Result<(), SyncError>>> = calendars
            .iter()
            .map(|id| Box::pin(self.sync_calendar(id)) as _)
            .collect();
        let results: Vec<Result<(), SyncError>> = stream::iter(pending)
            .buffer_unordered(PARALLEL_CALENDARS)
            .collect()
            .await;
        for error in results.into_iter().filter_map(Result::err) {
            tracing::warn!(account = %self.account_id, %error, "calendar sync failed");
            errors.push(error);
        }
        if let Err(error) = self.sync_tasks().await {
            tracing::warn!(account = %self.account_id, %error, "tasks sync failed");
            errors.push(error);
        }
        most_serious(errors).map_or(Ok(()), Err)
    }

    /// Whether settings and colours were last synced longer than `every` ago
    /// (or never), so the next round should fetch them.
    pub fn meta_due(&self, every: Duration) -> bool {
        self.meta_synced_at
            .lock()
            .expect("meta mutex poisoned")
            .is_none_or(|t| t.elapsed() >= every)
    }

    async fn sync_calendar_list(&self) -> Result<(), SyncError> {
        let mut reset = false;
        'restart: loop {
            let token = self.with_store(|s| s.calendar_list_sync_token(&self.account_id))?;
            let mut page_token: Option<String> = None;
            loop {
                match self
                    .client
                    .calendar_list_page(token.as_deref(), page_token.as_deref())
                    .await
                {
                    Ok(page) => {
                        let changes = self.with_store(|s| {
                            s.apply_calendar_list_page(
                                &self.account_id,
                                &page.items,
                                page.next_sync_token.as_deref(),
                            )
                        })?;
                        self.emit(changes);
                        match page.next_page_token {
                            Some(next) => page_token = Some(next),
                            None => return Ok(()),
                        }
                    }
                    Err(GoogleError::Gone) if !reset => {
                        reset = true;
                        tracing::info!(account = %self.account_id, "calendar list token expired; full re-sync");
                        self.with_store(|s| s.reset_account_sync(&self.account_id))?;
                        self.emit_everything();
                        continue 'restart;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }

    async fn sync_meta(&self) -> Result<(), SyncError> {
        let colors = self.fetch_colors().await?;
        let settings = self.fetch_settings().await?;
        self.with_store(|s| {
            s.put_colors(&self.account_id, &colors)?;
            s.put_settings(&self.account_id, &settings)
        })?;
        self.emit_everything();
        Ok(())
    }

    /// `colors.get` with its JSON kept. (`GoogleClient::colors` drops it.)
    async fn fetch_colors(&self) -> Result<Item<Colors>, GoogleError> {
        let url = calendar_url(&self.client, &["colors"])?;
        Item::from_value(self.client.get_json(url).await?)
    }

    /// Every page of `settings.list`, each item with its JSON kept.
    async fn fetch_settings(&self) -> Result<Vec<Item<Setting>>, GoogleError> {
        let mut settings = Vec::new();
        let mut page_token: Option<String> = None;
        loop {
            let mut url = calendar_url(&self.client, &["users", "me", "settings"])?;
            url.query_pairs_mut().append_pair("maxResults", "250");
            if let Some(token) = &page_token {
                url.query_pairs_mut().append_pair("pageToken", token);
            }
            let page: Page<Setting> = Page::from_value(self.client.get_json(url).await?)?;
            settings.extend(page.items);
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => return Ok(settings),
            }
        }
    }

    async fn sync_calendar(&self, calendar_id: &str) -> Result<(), SyncError> {
        let mut reset = false;
        'restart: loop {
            let token = self.with_store(|s| s.events_sync_token(&self.account_id, calendar_id))?;
            let mut page_token: Option<String> = None;
            loop {
                match self
                    .client
                    .events_page(calendar_id, token.as_deref(), page_token.as_deref())
                    .await
                {
                    Ok(page) => {
                        let changes = self.with_store(|s| {
                            s.apply_events_page(
                                &self.account_id,
                                calendar_id,
                                &page.items,
                                page.next_sync_token.as_deref(),
                            )
                        })?;
                        self.emit(changes);
                        match page.next_page_token {
                            Some(next) => page_token = Some(next),
                            None => return Ok(()),
                        }
                    }
                    Err(GoogleError::Gone) if !reset => {
                        reset = true;
                        tracing::info!(account = %self.account_id, calendar = %calendar_id, "events token expired; full re-sync");
                        self.with_store(|s| {
                            s.reset_calendar_events(&self.account_id, calendar_id)
                        })?;
                        self.emit_everything();
                        continue 'restart;
                    }
                    Err(e) if is_lost_access(&e) => {
                        // remove_calendar also clears the calendar-list token, so the
                        // next round refetches the whole list and can bring it back.
                        tracing::info!(account = %self.account_id, calendar = %calendar_id, error = %e, "lost access; removing calendar");
                        self.with_store(|s| s.remove_calendar(&self.account_id, calendar_id))?;
                        self.emit_everything();
                        return Ok(());
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }

    async fn sync_tasks(&self) -> Result<(), SyncError> {
        let lists = match self.client.task_lists().await {
            Ok(lists) => lists,
            Err(GoogleError::Forbidden { reason })
                if !TEMPORARY_FORBIDDEN.contains(&reason.as_str()) =>
            {
                tracing::warn!(account = %self.account_id, %reason, "Tasks API not available; skipping tasks");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let changes = self.with_store(|s| s.apply_task_lists(&self.account_id, &lists))?;
        self.emit(changes);
        for list in &lists {
            let list_id = list.fields.id.as_str();
            let since = self
                .with_store(|s| s.task_checkpoint(&self.account_id, list_id))?
                .map(|checkpoint| checkpoint - TASKS_OVERLAP);
            let mut page_token: Option<String> = None;
            // The newest `updated` across every page; stored only with the last
            // page so an interrupted listing starts again from the old checkpoint.
            let mut newest: Option<DateTime<Utc>> = None;
            loop {
                let page = match self
                    .client
                    .tasks_page(list_id, since, page_token.as_deref())
                    .await
                {
                    Ok(page) => page,
                    Err(GoogleError::NotFound) => break,
                    Err(e) => return Err(e.into()),
                };
                newest = page
                    .items
                    .iter()
                    .filter_map(|item| item.fields.updated)
                    .chain(newest)
                    .max();
                let last = page.next_page_token.is_none();
                let checkpoint = if last { newest } else { None };
                let changes = self.with_store(|s| {
                    s.apply_tasks_page(&self.account_id, list_id, &page.items, checkpoint)
                })?;
                self.emit(changes);
                match page.next_page_token {
                    Some(next) => page_token = Some(next),
                    None => break,
                }
            }
        }
        Ok(())
    }

    fn with_store<T>(
        &self,
        f: impl FnOnce(&mut Store) -> Result<T, StoreError>,
    ) -> Result<T, SyncError> {
        let mut store = self.store.lock().expect("store mutex poisoned");
        Ok(f(&mut store)?)
    }

    fn emit(&self, changes: ChangeSet) {
        if !changes.is_empty() {
            let _ = self.events.try_send(SyncEvent::Changed {
                account_id: self.account_id.clone(),
                changes,
            });
        }
    }

    fn emit_everything(&self) {
        self.emit(ChangeSet {
            everything: true,
            ..ChangeSet::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16) -> SyncError {
        GoogleError::Http {
            status,
            body: String::new(),
        }
        .into()
    }

    #[test]
    fn reauth_outranks_offline_which_outranks_the_rest() {
        let offline = || SyncError::from(GoogleError::Network("down".into()));
        let reauth = SyncError::from(GoogleError::InvalidGrant);
        assert!(
            most_serious(vec![http(500), offline(), reauth])
                .unwrap()
                .needs_reauth()
        );
        assert!(
            most_serious(vec![http(500), offline()])
                .unwrap()
                .is_offline()
        );
        let first = most_serious(vec![http(500), http(502)]).unwrap();
        assert!(matches!(
            first,
            SyncError::Google(GoogleError::Http { status: 500, .. })
        ));
        assert!(most_serious(Vec::new()).is_none());
    }

    #[test]
    fn quota_403s_are_not_lost_access() {
        let forbidden = |reason: &str| GoogleError::Forbidden {
            reason: reason.into(),
        };
        assert!(is_lost_access(&GoogleError::NotFound));
        assert!(is_lost_access(&forbidden("forbidden")));
        assert!(!is_lost_access(&forbidden("dailyLimitExceeded")));
        assert!(!is_lost_access(&forbidden("quotaExceeded")));
        assert!(!is_lost_access(&GoogleError::Gone));
    }
}
