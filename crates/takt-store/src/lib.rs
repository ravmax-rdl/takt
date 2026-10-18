//! SQLite cache of Google data for takt.
//!
//! Every row keeps Google's JSON in `raw_json`. The other columns are indexes
//! derived from it.

mod accounts;
mod convert;
mod error;
mod events;
mod query;
mod tasks;
mod types;

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use rusqlite_migration::{M, Migrations};

pub use error::StoreError;
pub use takt_recur::{Range, When};
pub use types::*;

pub struct Store {
    pub(crate) conn: Connection,
}

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(include_str!("../migrations/001_init.sql"))])
}

impl Store {
    /// Opens (creating if needed) the database the sync engine writes to.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::init(Connection::open_in_memory()?)
    }

    /// A reader for the UI. The database must already exist and be migrated.
    pub fn open_read_only(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.busy_timeout(Duration::from_secs(5))?;
        Ok(Self { conn })
    }

    fn init(mut conn: Connection) -> Result<Self, StoreError> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        migrations().to_latest(&mut conn)?;
        Ok(Self { conn })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn migrations_are_valid() {
        assert!(super::migrations().validate().is_ok());
    }
}
