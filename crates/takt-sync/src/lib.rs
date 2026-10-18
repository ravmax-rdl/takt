//! Background sync engine for takt.

mod engine;
mod messages;

pub use engine::{AccountEngine, PARALLEL_CALENDARS, SyncError};
pub use messages::{Command, SyncEvent, SyncState};
