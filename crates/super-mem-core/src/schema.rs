//! `SQLite` schema and connection initialization.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::{Durability, EngineOptions, Error, Result};

pub(crate) const SCHEMA_VERSION: u32 = 1;
/// `SQLite` application identifier (`SMEM`) used to distinguish stores from
/// unrelated files before destructive maintenance operations.
pub const APPLICATION_ID: u32 = 0x534D_454D;

/// Checks a database's identity without creating or modifying it.
///
/// # Errors
///
/// Returns an error if a recognized database's schema metadata cannot be read.
pub fn is_super_mem_database(path: impl AsRef<Path>) -> Result<bool> {
    if !path.as_ref().is_file() {
        return Ok(false);
    }
    let Ok(connection) = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return Ok(false);
    };
    let Ok(application_id) =
        connection.query_row("PRAGMA application_id", [], |row| row.get::<_, u32>(0))
    else {
        return Ok(false);
    };
    let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(application_id == APPLICATION_ID && (1..=SCHEMA_VERSION).contains(&version))
}

pub(crate) fn initialize(connection: &Connection, options: &EngineOptions) -> Result<()> {
    connection.busy_timeout(std::time::Duration::from_millis(options.busy_timeout_ms))?;
    connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;")?;

    // Verify identity before enabling WAL, which would otherwise mutate an
    // unrelated SQLite file supplied accidentally.
    let current: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let application_id: u32 =
        connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    let user_objects: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )?;
    if (current != 0 || application_id != 0 || user_objects != 0)
        && application_id != APPLICATION_ID
    {
        return Err(Error::Migration(
            "database is not a super-mem store (application_id mismatch)".into(),
        ));
    }
    if current > SCHEMA_VERSION {
        return Err(Error::Migration(format!(
            "database schema {current} is newer than supported schema {SCHEMA_VERSION}"
        )));
    }

    let journal_mode: String =
        connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
    // In-memory databases correctly report `memory` rather than `wal`.
    if journal_mode != "wal" && journal_mode != "memory" {
        return Err(Error::Migration(format!(
            "SQLite refused WAL mode and selected {journal_mode}"
        )));
    }
    match options.durability {
        Durability::Balanced => connection.execute_batch("PRAGMA synchronous=NORMAL;")?,
        Durability::Durable => connection.execute_batch("PRAGMA synchronous=FULL;")?,
    }

    if current == 0 {
        migrate_v1(connection)?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn migrate_v1(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            r"
            BEGIN IMMEDIATE;

            CREATE TABLE events (
                seq               INTEGER PRIMARY KEY AUTOINCREMENT,
                event_id          TEXT NOT NULL UNIQUE,
                namespace         TEXT NOT NULL,
                kind              TEXT NOT NULL,
                scope_json        TEXT NOT NULL,
                content           TEXT NOT NULL,
                attributes_json   TEXT NOT NULL,
                trust             TEXT NOT NULL,
                occurred_at_ms    INTEGER NOT NULL,
                ingested_at_ms    INTEGER NOT NULL,
                content_hash      TEXT NOT NULL,
                redaction_count   INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX events_namespace_seq ON events(namespace, seq DESC);
            CREATE INDEX events_hash ON events(namespace, content_hash);

            CREATE TABLE memory_heads (
                docid             INTEGER PRIMARY KEY AUTOINCREMENT,
                memory_id         TEXT NOT NULL UNIQUE,
                namespace         TEXT NOT NULL,
                scope_key         TEXT NOT NULL,
                workspace_id      TEXT,
                repo_id           TEXT,
                branch            TEXT,
                session_id        TEXT,
                kind              TEXT NOT NULL,
                state             TEXT NOT NULL,
                canonical_key     TEXT,
                head_revision     INTEGER NOT NULL,
                importance        REAL NOT NULL CHECK(importance >= 0 AND importance <= 1),
                confidence        REAL NOT NULL CHECK(confidence >= 0 AND confidence <= 1),
                trust             TEXT NOT NULL,
                valid_from_ms     INTEGER,
                valid_until_ms    INTEGER,
                expires_at_ms     INTEGER,
                created_at_ms     INTEGER NOT NULL,
                updated_at_ms     INTEGER NOT NULL,
                created_seq       INTEGER NOT NULL REFERENCES events(seq),
                updated_seq       INTEGER NOT NULL REFERENCES events(seq)
            );
            CREATE INDEX memory_heads_namespace_state
                ON memory_heads(namespace, state, updated_seq DESC);
            CREATE INDEX memory_heads_repo
                ON memory_heads(namespace, repo_id, branch, updated_seq DESC);
            CREATE INDEX memory_heads_canonical
                ON memory_heads(namespace, scope_key, kind, canonical_key)
                WHERE canonical_key IS NOT NULL;

            CREATE TABLE memory_revisions (
                memory_id         TEXT NOT NULL REFERENCES memory_heads(memory_id),
                revision          INTEGER NOT NULL,
                title             TEXT NOT NULL,
                body              TEXT NOT NULL,
                attributes_json   TEXT NOT NULL,
                scope_json        TEXT NOT NULL,
                content_hash      TEXT NOT NULL,
                recorded_at_ms    INTEGER NOT NULL,
                retired_at_ms     INTEGER,
                recorded_seq      INTEGER NOT NULL REFERENCES events(seq),
                PRIMARY KEY(memory_id, revision)
            );
            CREATE INDEX memory_revisions_hash ON memory_revisions(content_hash);

            CREATE TABLE memory_evidence (
                memory_id         TEXT NOT NULL,
                revision          INTEGER NOT NULL,
                event_id          TEXT NOT NULL REFERENCES events(event_id),
                span_start        INTEGER,
                span_end          INTEGER,
                relation          TEXT NOT NULL,
                PRIMARY KEY(memory_id, revision, event_id, relation),
                FOREIGN KEY(memory_id, revision)
                    REFERENCES memory_revisions(memory_id, revision)
            );

            CREATE TABLE memory_tags (
                memory_id         TEXT NOT NULL,
                revision          INTEGER NOT NULL,
                tag               TEXT NOT NULL,
                normalized        TEXT NOT NULL,
                PRIMARY KEY(memory_id, revision, normalized),
                FOREIGN KEY(memory_id, revision)
                    REFERENCES memory_revisions(memory_id, revision)
            );
            CREATE INDEX memory_tags_normalized ON memory_tags(normalized, memory_id);

            CREATE TABLE entities (
                entity_id         INTEGER PRIMARY KEY AUTOINCREMENT,
                namespace         TEXT NOT NULL,
                kind              TEXT NOT NULL,
                canonical         TEXT NOT NULL,
                display           TEXT NOT NULL,
                UNIQUE(namespace, kind, canonical)
            );
            CREATE TABLE memory_entities (
                memory_id         TEXT NOT NULL,
                revision          INTEGER NOT NULL,
                entity_id         INTEGER NOT NULL REFERENCES entities(entity_id),
                PRIMARY KEY(memory_id, revision, entity_id),
                FOREIGN KEY(memory_id, revision)
                    REFERENCES memory_revisions(memory_id, revision)
            );
            CREATE INDEX entities_canonical ON entities(namespace, canonical);

            CREATE TABLE artifacts (
                artifact_id       INTEGER PRIMARY KEY AUTOINCREMENT,
                namespace         TEXT NOT NULL,
                repo_id           TEXT NOT NULL,
                path              TEXT NOT NULL,
                symbol            TEXT NOT NULL DEFAULT '',
                content_hash      TEXT NOT NULL DEFAULT '',
                git_oid           TEXT NOT NULL DEFAULT '',
                language          TEXT NOT NULL DEFAULT '',
                UNIQUE(namespace, repo_id, path, symbol, content_hash, git_oid)
            );
            CREATE TABLE memory_artifacts (
                memory_id         TEXT NOT NULL,
                revision          INTEGER NOT NULL,
                artifact_id       INTEGER NOT NULL REFERENCES artifacts(artifact_id),
                PRIMARY KEY(memory_id, revision, artifact_id),
                FOREIGN KEY(memory_id, revision)
                    REFERENCES memory_revisions(memory_id, revision)
            );
            CREATE INDEX artifacts_lookup ON artifacts(namespace, repo_id, path, symbol);

            CREATE TABLE memory_links (
                link_id           TEXT PRIMARY KEY,
                source_memory_id  TEXT NOT NULL REFERENCES memory_heads(memory_id),
                target_memory_id  TEXT NOT NULL REFERENCES memory_heads(memory_id),
                relation          TEXT NOT NULL,
                weight            INTEGER NOT NULL CHECK(weight >= 0 AND weight <= 1000),
                created_event_id  TEXT NOT NULL REFERENCES events(event_id),
                created_at_ms     INTEGER NOT NULL,
                UNIQUE(source_memory_id, target_memory_id, relation)
            );
            CREATE INDEX memory_links_target ON memory_links(target_memory_id, relation);

            CREATE TABLE event_memories (
                event_id          TEXT NOT NULL REFERENCES events(event_id),
                memory_id         TEXT NOT NULL REFERENCES memory_heads(memory_id),
                PRIMARY KEY(event_id, memory_id)
            );

            CREATE TABLE feedback (
                feedback_id       INTEGER PRIMARY KEY AUTOINCREMENT,
                query_id          TEXT,
                memory_id         TEXT NOT NULL REFERENCES memory_heads(memory_id),
                signal            TEXT NOT NULL,
                note              TEXT,
                created_at_ms     INTEGER NOT NULL
            );
            CREATE INDEX feedback_memory ON feedback(memory_id, signal);

            CREATE TABLE idempotency (
                namespace         TEXT NOT NULL,
                operation         TEXT NOT NULL,
                idempotency_key   TEXT NOT NULL,
                request_hash      TEXT NOT NULL,
                receipt_json      TEXT NOT NULL,
                created_at_ms     INTEGER NOT NULL,
                PRIMARY KEY(namespace, operation, idempotency_key)
            );

            CREATE VIRTUAL TABLE memory_fts USING fts5(
                title,
                body,
                tags,
                entities,
                paths,
                tokenize = 'unicode61 remove_diacritics 2'
            );

            PRAGMA application_id=1397572941;
            PRAGMA user_version=1;
            COMMIT;
            ",
        )
        .map_err(|error| Error::Migration(error.to_string()))?;
    Ok(())
}
