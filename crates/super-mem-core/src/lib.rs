#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]

//! Evidence-first, repository-aware memory for coding agents.
//!
//! The crate deliberately keeps its canonical state in one `SQLite` database.
//! Events and memory revisions are immutable; search indexes are derived state.

mod applicability;
mod artifacts;
mod engine;
mod error;
mod git;
mod ranking;
mod redaction;
mod schema;
mod search;
mod types;

pub use applicability::classify_applicability;
pub use artifacts::{capture_artifact_paths, capture_changed_artifacts};
pub use engine::MemoryEngine;
pub use error::{Error, Result};
pub use git::{canonical_path_digest, compare_revisions, discover_repository, normalize_remote};
pub use redaction::{Redaction, Redactor};
pub use schema::{APPLICATION_ID, is_super_mem_database};
pub use types::*;
