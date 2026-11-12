//! Background sync engine for takt.

mod engine;
mod messages;
mod runtime;

pub use engine::{AccountEngine, PARALLEL_CALENDARS, SyncError};
pub use messages::{
    Command, InstanceOf, SyncEvent, SyncState, WriteError, WriteOp, WriteOutcome, WriteRequest,
};
pub use runtime::{Intervals, SyncConfig, SyncHandle, start};
