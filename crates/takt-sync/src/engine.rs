//! One sync round for one account, and what to do when Google says no.

use std::sync::{Arc, Mutex};

use chrono::TimeDelta;
use futures::stream::{self, StreamExt};
use takt_google::{GoogleClient, GoogleError};
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
}

pub struct AccountEngine {
    account_id: String,
    client: GoogleClient,
    store: Arc<Mutex<Store>>,
    events: async_channel::Sender<SyncEvent>,
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
        }
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// Calendar list, then settings and colours when asked, then every
    /// non-hidden calendar's events (primary first, four at a time), then tasks.
    pub async fn round(&self, with_meta: bool) -> Result<(), SyncError> {
        self.sync_calendar_list().await?;
        if with_meta {
            self.sync_meta().await?;
        }
        let calendars: Vec<String> = self
            .with_store(|s| s.calendars(Some(&self.account_id)))?
            .into_iter()
            .filter(|c| !c.hidden)
            .map(|c| c.calendar_id)
            .collect();
        let results: Vec<Result<(), SyncError>> =
            stream::iter(calendars.iter().map(|id| self.sync_calendar(id)))
                .buffer_unordered(PARALLEL_CALENDARS)
                .collect()
                .await;
        results.into_iter().collect::<Result<(), _>>()?;
        self.sync_tasks().await
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
        let colors = self.client.colors().await?;
        let settings = self.client.settings().await?;
        self.with_store(|s| {
            s.put_colors(&self.account_id, &colors)?;
            s.put_settings(&self.account_id, &settings)
        })?;
        self.emit_everything();
        Ok(())
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
                    Err(GoogleError::NotFound | GoogleError::Forbidden { .. }) => {
                        tracing::info!(account = %self.account_id, calendar = %calendar_id, "lost access; removing calendar");
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
            Err(GoogleError::Forbidden { reason }) => {
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
                let changes = self
                    .with_store(|s| s.apply_tasks_page(&self.account_id, list_id, &page.items))?;
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
