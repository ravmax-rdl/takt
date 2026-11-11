//! One sync round for one account, and what to do when Google says no.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Utc};
use futures::stream::{self, StreamExt};
use serde_json::Value;
use takt_google::model::{EventFields, Item};
use takt_google::{GoogleClient, GoogleError, SendUpdates, new_event_id};
use takt_store::{ChangeSet, Store, StoreError};

use crate::{SyncEvent, WriteError, WriteOp, WriteOutcome, WriteRequest};

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

/// The error a round reports: one of the most serious, the earliest among equals.
fn most_serious(errors: Vec<SyncError>) -> Option<SyncError> {
    errors.into_iter().min_by_key(SyncError::severity)
}

/// 403 reasons that mean "not now" rather than "no access".
const TEMPORARY_FORBIDDEN: &[&str] = &["dailyLimitExceeded", "quotaExceeded"];

/// 403 reasons about the app or the grant (API switched off, scope missing),
/// not about one calendar: every calendar would fail the same way, so none is
/// removed for them.
const ACCOUNT_FORBIDDEN: &[&str] = &[
    "accessNotConfigured",
    "insufficientPermissions",
    "SERVICE_DISABLED",
    "ACCESS_TOKEN_SCOPE_INSUFFICIENT",
];

fn is_lost_access(error: &GoogleError) -> bool {
    match error {
        GoogleError::NotFound => true,
        GoogleError::Forbidden { reason } => {
            let reason = reason.as_str();
            !TEMPORARY_FORBIDDEN.contains(&reason) && !ACCOUNT_FORBIDDEN.contains(&reason)
        }
        _ => false,
    }
}

/// Whether a round should stop: every later request would fail the same way.
fn ends_round(error: &SyncError) -> bool {
    error.needs_reauth() || error.is_offline()
}

pub struct AccountEngine {
    account_id: String,
    client: GoogleClient,
    store: Arc<Mutex<Store>>,
    events: async_channel::Sender<SyncEvent>,
    meta_synced_at: Mutex<Option<Instant>>,
    /// One lock per calendar, held while a page of its events is fetched and
    /// stored and while a write to it runs. Without it a page fetched before
    /// a write could be stored after the write's result and undo it on screen
    /// until the next round.
    calendar_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
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
            calendar_locks: Mutex::default(),
        }
    }

    fn calendar_lock(&self, calendar_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.calendar_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(calendar_id.to_owned())
            .or_default()
            .clone()
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// Calendar list, then settings and colours when asked, then every
    /// non-hidden calendar's events (primary first, four at a time), then tasks.
    ///
    /// One failing part does not stop the others. A revoked grant or lost
    /// connection on the calendar list, meta or any calendar ends the round
    /// there, since every later request would fail the same way. A 403 on the
    /// calendar list is account-wide, so the events step is skipped rather
    /// than letting every calendar fail (or be removed) for it. Otherwise the
    /// round runs to the end and returns the most serious error: re-auth, then
    /// offline, then the first other one.
    pub async fn round(&self, with_meta: bool) -> Result<(), SyncError> {
        let mut errors = Vec::new();
        let mut skip_events = false;
        if let Err(error) = self.sync_calendar_list().await {
            if ends_round(&error) {
                return Err(error);
            }
            skip_events = matches!(error, SyncError::Google(GoogleError::Forbidden { .. }));
            tracing::warn!(account = %self.account_id, %error, skip_events, "calendar list sync failed; using stored calendars");
            errors.push(error);
        }
        if with_meta {
            match self.sync_meta().await {
                Ok(()) => {
                    *self
                        .meta_synced_at
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now())
                }
                Err(error) if ends_round(&error) => return Err(error),
                Err(error) => {
                    tracing::warn!(account = %self.account_id, %error, "settings and colours sync failed");
                    errors.push(error);
                }
            }
        }
        if !skip_events {
            self.sync_events(&mut errors).await?;
            if errors.iter().any(ends_round) {
                return most_serious(errors).map_or(Ok(()), Err);
            }
        }
        self.sync_tasks(&mut errors).await;
        most_serious(errors).map_or(Ok(()), Err)
    }

    /// Every non-hidden calendar's events, primary first, four at a time.
    /// Per-calendar errors go to `errors`.
    async fn sync_events(&self, errors: &mut Vec<SyncError>) -> Result<(), SyncError> {
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
        Ok(())
    }

    /// Whether settings and colours were last synced longer than `every` ago
    /// (or never), so the next round should fetch them.
    pub fn meta_due(&self, every: Duration) -> bool {
        self.meta_synced_at
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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
                let lock = self.calendar_lock(calendar_id);
                let _held = lock.lock().await;
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

    /// Task lists, then each list's tasks. One failing list does not stop
    /// the others unless the error ends the round. Errors go to `errors`.
    async fn sync_tasks(&self, errors: &mut Vec<SyncError>) {
        let lists = match self.client.task_lists().await {
            Ok(lists) => lists,
            Err(GoogleError::Forbidden { reason })
                if !TEMPORARY_FORBIDDEN.contains(&reason.as_str()) =>
            {
                tracing::warn!(account = %self.account_id, %reason, "Tasks API not available; skipping tasks");
                return;
            }
            Err(error) => {
                tracing::warn!(account = %self.account_id, %error, "task lists sync failed");
                return errors.push(error.into());
            }
        };
        match self.with_store(|s| s.apply_task_lists(&self.account_id, &lists)) {
            Ok(changes) => self.emit(changes),
            Err(error) => {
                tracing::warn!(account = %self.account_id, %error, "cannot store task lists");
                return errors.push(error);
            }
        }
        for list in &lists {
            let list_id = list.fields.id.as_str();
            if let Err(error) = self.sync_task_list(list_id).await {
                tracing::warn!(account = %self.account_id, list = %list_id, %error, "task list sync failed");
                let stop = ends_round(&error);
                errors.push(error);
                if stop {
                    return;
                }
            }
        }
    }

    async fn sync_task_list(&self, list_id: &str) -> Result<(), SyncError> {
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
                Err(GoogleError::NotFound) => return Ok(()),
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
                None => return Ok(()),
            }
        }
    }

    /// Sends one change to Google and stores what Google answers, so the UI
    /// shows the result without waiting for the next round.
    pub async fn write(&self, request: &WriteRequest) -> Result<WriteOutcome, WriteError> {
        let calendar_id = request.calendar_id.as_str();
        let send = if request.notify_guests {
            SendUpdates::All
        } else {
            SendUpdates::None
        };
        // Always in the same order, so two writes cannot wait on each other.
        let mut calendars = vec![calendar_id];
        if let WriteOp::Update {
            move_to: Some(destination),
            ..
        } = &request.op
        {
            calendars.push(destination);
        }
        calendars.sort_unstable();
        calendars.dedup();
        let locks: Vec<_> = calendars.iter().map(|id| self.calendar_lock(id)).collect();
        let mut held = Vec::with_capacity(locks.len());
        for lock in &locks {
            held.push(lock.lock().await);
        }

        match &request.op {
            WriteOp::Create { body } => {
                let mut body = body.clone();
                if !body.is_object() {
                    return Err(WriteError::Rejected(
                        "the new event is not an object".into(),
                    ));
                }
                if body.get("id").and_then(Value::as_str).is_none() {
                    body["id"] = new_event_id().into();
                }
                let id = body["id"].as_str().unwrap_or_default().to_owned();
                let created = match self.client.insert_event(calendar_id, &body, send).await {
                    Ok(created) => created,
                    // The ID is ours and new, so it can only exist because an
                    // earlier attempt of this same insert reached Google.
                    Err(GoogleError::Conflict) => self
                        .client
                        .get_event(calendar_id, &id)
                        .await
                        .map_err(write_error)?,
                    Err(error) => return Err(write_error(error)),
                };
                self.store_written(calendar_id, &created)?;
                Ok(WriteOutcome {
                    calendar_id: calendar_id.to_owned(),
                    event_id: created.fields.id,
                })
            }
            WriteOp::Update {
                event_id,
                etag,
                patch,
                base,
                move_to,
            } => {
                if patch.as_object().is_some_and(|fields| !fields.is_empty()) {
                    self.patch(calendar_id, event_id, patch, base, etag.as_deref(), send)
                        .await?;
                }
                let Some(destination) = move_to.as_deref().filter(|d| *d != calendar_id) else {
                    return Ok(WriteOutcome {
                        calendar_id: calendar_id.to_owned(),
                        event_id: event_id.clone(),
                    });
                };
                let moved = self
                    .client
                    .move_event(calendar_id, event_id, destination, send)
                    .await
                    .map_err(|error| self.failed(error, calendar_id, event_id))?;
                self.record_delete(calendar_id, event_id, None)?;
                self.store_written(destination, &moved)?;
                Ok(WriteOutcome {
                    calendar_id: destination.to_owned(),
                    event_id: moved.fields.id,
                })
            }
            WriteOp::Delete { event_id, instance } => {
                match self
                    .client
                    .delete_event(calendar_id, event_id, None, send)
                    .await
                {
                    // Already deleted elsewhere: the same end state.
                    Ok(()) | Err(GoogleError::Gone | GoogleError::NotFound) => {}
                    Err(error) => return Err(write_error(error)),
                }
                let instance = instance
                    .as_ref()
                    .map(|i| (i.series_id.as_str(), i.original_start));
                self.record_delete(calendar_id, event_id, instance)?;
                Ok(WriteOutcome {
                    calendar_id: calendar_id.to_owned(),
                    event_id: event_id.clone(),
                })
            }
        }
    }

    /// `events.patch` guarded by the etag the edit started from. When Google
    /// has a newer version, its copy is stored, and the patch is sent again
    /// against it only if nobody else changed the fields being written.
    async fn patch(
        &self,
        calendar_id: &str,
        event_id: &str,
        patch: &Value,
        base: &Value,
        etag: Option<&str>,
        send: SendUpdates,
    ) -> Result<(), WriteError> {
        let first = self
            .client
            .patch_event(calendar_id, event_id, patch, etag, send)
            .await;
        let error = match first {
            Ok(updated) => return self.store_written(calendar_id, &updated),
            Err(error) => error,
        };
        if !matches!(error, GoogleError::PreconditionFailed) {
            return Err(self.failed(error, calendar_id, event_id));
        }
        let remote = self
            .client
            .get_event(calendar_id, event_id)
            .await
            .map_err(|error| self.failed(error, calendar_id, event_id))?;
        self.store_written(calendar_id, &remote)?;
        if remote.fields.is_cancelled() {
            return Err(WriteError::Gone);
        }
        if includes(&remote.raw, patch) {
            // Google already has these values (the first attempt landed, or
            // someone made the same change).
            return Ok(());
        }
        if !untouched(patch, base, &remote.raw) {
            return Err(WriteError::Conflict);
        }
        let again = self
            .client
            .patch_event(
                calendar_id,
                event_id,
                patch,
                remote.fields.etag.as_deref(),
                send,
            )
            .await;
        match again {
            Ok(updated) => self.store_written(calendar_id, &updated),
            Err(error) => Err(self.failed(error, calendar_id, event_id)),
        }
    }

    fn store_written(
        &self,
        calendar_id: &str,
        event: &Item<EventFields>,
    ) -> Result<(), WriteError> {
        let changes = self
            .with_store(|s| s.apply_event(&self.account_id, calendar_id, event))
            .map_err(cache_error)?;
        self.emit(changes);
        Ok(())
    }

    fn record_delete(
        &self,
        calendar_id: &str,
        event_id: &str,
        instance: Option<(&str, takt_store::When)>,
    ) -> Result<(), WriteError> {
        let changes = self
            .with_store(|s| s.mark_event_deleted(&self.account_id, calendar_id, event_id, instance))
            .map_err(cache_error)?;
        self.emit(changes);
        Ok(())
    }

    /// The error to report for a failed write to an existing event. An event
    /// Google no longer has is removed from the cache on the way.
    fn failed(&self, error: GoogleError, calendar_id: &str, event_id: &str) -> WriteError {
        let error = write_error(error);
        if error == WriteError::Gone
            && let Err(error) = self.record_delete(calendar_id, event_id, None)
        {
            tracing::warn!(account = %self.account_id, %error, "cannot drop a deleted event");
        }
        error
    }

    fn with_store<T>(
        &self,
        f: impl FnOnce(&mut Store) -> Result<T, StoreError>,
    ) -> Result<T, SyncError> {
        let mut store = self.store.lock().unwrap_or_else(PoisonError::into_inner);
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

fn write_error(error: GoogleError) -> WriteError {
    match error {
        GoogleError::Network(_) => WriteError::Offline,
        GoogleError::InvalidGrant => WriteError::NotSignedIn,
        GoogleError::NotFound | GoogleError::Gone => WriteError::Gone,
        GoogleError::PreconditionFailed => WriteError::Conflict,
        GoogleError::Forbidden { reason } => {
            WriteError::Rejected(format!("Google does not allow this change ({reason})"))
        }
        other => WriteError::Rejected(other.to_string()),
    }
}

/// Google took the write but the cache did not; the next round repairs it.
fn cache_error(error: SyncError) -> WriteError {
    WriteError::Rejected(format!(
        "saved in Google, but the local cache could not be updated: {error}"
    ))
}

/// Whether `remote` already holds everything `patch` would set. In a patch
/// a `null` means "absent".
fn includes(remote: &Value, patch: &Value) -> bool {
    match patch {
        Value::Null => remote.is_null(),
        Value::Object(fields) => fields
            .iter()
            .all(|(key, value)| includes(remote.get(key).unwrap_or(&Value::Null), value)),
        other => remote == other,
    }
}

/// Whether every top-level field `patch` writes is the same in `remote` as
/// it was in `base`, i.e. nobody else changed what this patch changes.
fn untouched(patch: &Value, base: &Value, remote: &Value) -> bool {
    patch.as_object().is_some_and(|fields| {
        fields.keys().all(|key| {
            base.get(key).unwrap_or(&Value::Null) == remote.get(key).unwrap_or(&Value::Null)
        })
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_patch_is_included_when_google_already_has_its_values() {
        let remote = json!({"summary": "Lunch", "start": {"dateTime": "2026-10-19T12:00:00Z"}});
        assert!(includes(&remote, &json!({"summary": "Lunch"})));
        assert!(includes(&remote, &json!({"location": null})));
        assert!(includes(
            &remote,
            &json!({"start": {"dateTime": "2026-10-19T12:00:00Z", "date": null}})
        ));
        assert!(!includes(&remote, &json!({"summary": "Dinner"})));
        assert!(!includes(&remote, &json!({"summary": null})));
        assert!(!includes(
            &remote,
            &json!({"start": {"date": "2026-10-19"}})
        ));
    }

    #[test]
    fn untouched_looks_only_at_the_patched_fields() {
        let base = json!({"summary": "Lunch", "location": null});
        let patch = json!({"summary": "Late lunch", "location": "Cafe"});
        let elsewhere = json!({"summary": "Lunch", "description": "new notes"});
        assert!(untouched(&patch, &base, &elsewhere));
        let same_field = json!({"summary": "Brunch"});
        assert!(!untouched(&patch, &base, &same_field));
        let location_added = json!({"summary": "Lunch", "location": "Office"});
        assert!(!untouched(&patch, &base, &location_added));
        assert!(!untouched(&json!("not an object"), &base, &elsewhere));
    }

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
        for reason in [
            "accessNotConfigured",
            "insufficientPermissions",
            "SERVICE_DISABLED",
            "ACCESS_TOKEN_SCOPE_INSUFFICIENT",
        ] {
            assert!(!is_lost_access(&forbidden(reason)), "{reason}");
        }
        assert!(!is_lost_access(&GoogleError::Gone));
    }
}
