use chrono::{DateTime, Utc};
use serde_json::Value;
use takt_store::{ChangeSet, When};

/// UI → sync.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    AddAccount,
    RemoveAccount {
        account_id: String,
    },
    SyncNow,
    /// Shows or hides a calendar in takt only; nothing is sent to Google.
    SetCalendarVisible {
        account_id: String,
        calendar_id: String,
        visible: bool,
    },
    /// Sends one change to Google. Answered with [`SyncEvent::WriteDone`].
    Write(Box<WriteRequest>),
    WindowFocus(bool),
    Shutdown,
}

/// One change to one event.
#[derive(Debug, Clone, PartialEq)]
pub struct WriteRequest {
    /// Chosen by the sender and echoed in [`SyncEvent::WriteDone`].
    pub id: u64,
    pub account_id: String,
    pub calendar_id: String,
    pub op: WriteOp,
    /// Whether Google emails the event's guests about it.
    pub notify_guests: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WriteOp {
    /// `events.insert` with this body. An `id` is added when it has none.
    Create { body: Value },
    Update {
        /// The event's ID, or an occurrence's instance ID.
        event_id: String,
        /// The etag the edit started from. `None` for an occurrence that has
        /// no row of its own yet.
        etag: Option<String>,
        /// The changed top-level fields, as `events.patch` takes them. May be
        /// an empty object when only the calendar changes.
        patch: Value,
        /// The same fields as they were when the edit started (`null` where
        /// absent). On an etag mismatch the patch is sent again only if none
        /// of these changed in Google meanwhile.
        base: Value,
        /// Move the event to this calendar of the same account afterwards.
        move_to: Option<String>,
    },
    /// Deletes are not guarded by an etag: a changed event is still deleted.
    Delete {
        event_id: String,
        /// For an occurrence of a series: which series, and its original start.
        instance: Option<InstanceOf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceOf {
    pub series_id: String,
    pub original_start: When,
}

/// Where a written event now is. `event_id` is new for a create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteOutcome {
    pub calendar_id: String,
    pub event_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// The event changed in Google in the fields being written. The cache
    /// now holds Google's version; nothing was written.
    #[error("the event changed in Google while you were editing it")]
    Conflict,
    /// The event no longer exists in Google. It was removed from the cache.
    #[error("the event no longer exists in Google")]
    Gone,
    #[error("no connection to Google; nothing was changed")]
    Offline,
    /// The account is signed out, or its sync has not started.
    #[error("this account is not signed in")]
    NotSignedIn,
    /// Google refused the write; its own message.
    #[error("{0}")]
    Rejected(String),
}

/// Sync → UI. Never carries event data: the UI re-reads the store.
#[derive(Debug, Clone, PartialEq)]
pub enum SyncEvent {
    Changed {
        account_id: String,
        changes: ChangeSet,
    },
    Status {
        account_id: String,
        state: SyncState,
    },
    AuthNeeded {
        account_id: String,
    },
    AccountsChanged,
    SignInFailed(String),
    /// The answer to [`Command::Write`]. On success the store already holds
    /// the result and a `Changed` event was sent before this one.
    WriteDone {
        id: u64,
        result: Result<WriteOutcome, WriteError>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    Synced(DateTime<Utc>),
    Syncing,
    Offline(DateTime<Utc>),
    Error(String),
    NeedsReauth,
}
