//! Error types exposed by the memory engine.

use thiserror::Error;

/// Result type used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// A failure produced by the memory engine.
#[derive(Debug, Error)]
pub enum Error {
    /// `SQLite` rejected an operation.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// JSON serialization or deserialization failed.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    /// Filesystem I/O failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// A caller supplied an invalid request.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// The requested object does not exist.
    #[error("{kind} not found: {id}")]
    NotFound {
        /// Object category.
        kind: &'static str,
        /// Stable identifier supplied by the caller.
        id: String,
    },
    /// Existing state prevents the requested change.
    #[error("conflict: {0}")]
    Conflict(String),
    /// A mutex was poisoned by a prior panic.
    #[error("database lock is poisoned")]
    PoisonedLock,
    /// The database schema could not be initialized or upgraded.
    #[error("schema migration failed: {0}")]
    Migration(String),
}
