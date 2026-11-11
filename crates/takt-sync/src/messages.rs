use chrono::{DateTime, Utc};
use takt_store::ChangeSet;

/// UI → sync.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    WindowFocus(bool),
    Shutdown,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    Synced(DateTime<Utc>),
    Syncing,
    Offline(DateTime<Utc>),
    Error(String),
    NeedsReauth,
}
