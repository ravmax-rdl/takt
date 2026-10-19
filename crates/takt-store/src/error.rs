#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
    #[error(transparent)]
    Migration(#[from] rusqlite_migration::Error),
    #[error("stored JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("stored value is invalid: {0}")]
    Corrupt(String),
    #[error("cannot create the database directory: {0}")]
    Io(String),
}
