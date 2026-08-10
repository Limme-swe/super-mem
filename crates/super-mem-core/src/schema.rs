//! `SQLite` schema and connection initialization.

use std::{
    fs::File,
    io::{ErrorKind, Read},
    path::Path,
    thread,
    time::{Duration, Instant},
};

use rusqlite::{Connection, ErrorCode, OpenFlags};

use crate::{Durability, EngineOptions, Error, Result};

pub(crate) const SCHEMA_VERSION: u32 = 6;
/// `SQLite` application identifier (`SMEM`) used to distinguish stores from
/// unrelated files before destructive maintenance operations.
pub const APPLICATION_ID: u32 = 0x534D_454D;

/// Checks a database's identity without creating or modifying it.
///
/// # Errors
///
/// Returns an error if a recognized database's schema metadata cannot be read.
pub fn is_super_mem_database(path: impl AsRef<Path>) -> Result<bool> {
    let path = path.as_ref();
    if !path.is_file() {
        return Ok(false);
    }

    // Ask SQLite first so an open WAL is part of the view. The main database
    // is opened read-only, so this cannot perform rollback-journal recovery
    // while a destructive maintenance command identifies the store.
    if let Ok(connection) = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        let application_id =
            connection.query_row("PRAGMA application_id", [], |row| row.get::<_, u32>(0));
        let version = connection.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0));
        if let (Ok(application_id), Ok(version)) = (application_id, version) {
            return Ok(application_id == APPLICATION_ID && (1..=SCHEMA_VERSION).contains(&version));
        }
    }

    // The main header can be stale whenever a WAL exists. If SQLite could not
    // read the live view, fail closed instead of trusting that stale header.
    let mut wal_path = path.as_os_str().to_os_string();
    wal_path.push("-wal");
    if Path::new(&wal_path).exists() {
        return Ok(false);
    }

    let Ok(mut file) = File::open(path) else {
        return Ok(false);
    };
    // A damaged or incomplete crash journal can make even a read-only SQLite
    // open fail. Fall back to immutable main-header fields so the store remains
    // identifiable for purge without attempting journal recovery.
    let mut header = [0_u8; 72];
    if let Err(error) = file.read_exact(&mut header) {
        return if error.kind() == ErrorKind::UnexpectedEof {
            Ok(false)
        } else {
            Err(error.into())
        };
    }
    if &header[..16] != b"SQLite format 3\0" {
        return Ok(false);
    }
    let version = u32::from_be_bytes([header[60], header[61], header[62], header[63]]);
    let application_id = u32::from_be_bytes([header[68], header[69], header[70], header[71]]);
    Ok(application_id == APPLICATION_ID && (1..=SCHEMA_VERSION).contains(&version))
}

pub(crate) fn initialize(connection: &Connection, options: &EngineOptions) -> Result<()> {
    let busy_timeout = Duration::from_millis(options.busy_timeout_ms);
    connection.busy_timeout(busy_timeout)?;
    connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;")?;

    // Verify identity before enabling WAL, which would otherwise mutate an
    // unrelated SQLite file supplied accidentally. The same check is repeated
    // after acquiring the migration writer lock because another process may
    // initialize an empty file between these two steps.
    validate_identity(connection)?;

    let journal_mode = retry_busy(busy_timeout, || {
        let current: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        if current == "wal" || current == "memory" {
            Ok(current)
        } else {
            connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        }
    })?;
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

    retry_busy(busy_timeout, || {
        connection.execute_batch("BEGIN IMMEDIATE;")
    })?;
    let migration = (|| {
        let current = validate_identity(connection)?;
        if current == 0 {
            migrate_v1(connection)?;
        }
        if current < 2 {
            migrate_v2(connection)?;
        }
        if current < 3 {
            migrate_v3(connection)?;
        }
        if current < 4 {
            migrate_v4(connection)?;
        }
        if current < 5 {
            migrate_v5(connection)?;
        }
        if current < 6 {
            migrate_v6(connection)?;
        }
        Ok(())
    })();
    match migration {
        Ok(()) => {
            connection.execute_batch("COMMIT;")?;
            Ok(())
        }
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK;");
            Err(error)
        }
    }
}

fn retry_busy<T>(
    timeout: Duration,
    mut operation: impl FnMut() -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    let started = Instant::now();
    let mut delay = Duration::from_millis(1);
    loop {
        match operation() {
            Err(error) if is_busy(&error) && started.elapsed() < timeout => {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(error);
                }
                thread::sleep(delay.min(remaining));
                delay = delay.saturating_mul(2).min(Duration::from_millis(25));
            }
            result => return result,
        }
    }
}

fn is_busy(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(
                failure.code,
                ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
            )
    )
}

fn validate_identity(connection: &Connection) -> Result<u32> {
    // Read all identity fields in one SQLite statement. Separate PRAGMA and
    // schema queries can straddle another process's migration commit and
    // observe an impossible mixture of the old header and new schema.
    let (current, application_id, user_objects) = connection.query_row(
        r"
        SELECT
            (SELECT user_version FROM pragma_user_version),
            (SELECT application_id FROM pragma_application_id),
            (SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%')
        ",
        [],
        |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
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
    Ok(current)
}

#[allow(clippy::too_many_lines)]
fn migrate_v1(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            r"
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
            ",
        )
        .map_err(|error| Error::Migration(error.to_string()))?;
    Ok(())
}

fn migrate_v2(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            r"
            CREATE INDEX memory_entities_lookup
                ON memory_entities(entity_id,memory_id,revision);
            CREATE INDEX memory_artifacts_memory_lookup
                ON memory_artifacts(artifact_id,memory_id,revision);
            DROP TABLE memory_fts;
            CREATE VIRTUAL TABLE memory_fts USING fts5(
                title,
                body,
                tags,
                entities,
                paths,
                tokenize = 'unicode61 remove_diacritics 2',
                content = '',
                contentless_delete = 1
            );
            INSERT INTO memory_fts(rowid,title,body,tags,entities,paths)
            SELECT h.docid,
                   r.title,
                   r.body,
                   coalesce((SELECT group_concat(t.tag, ' ') FROM memory_tags t
                             WHERE t.memory_id=h.memory_id AND t.revision=h.head_revision), ''),
                   coalesce((SELECT group_concat(e.canonical || ' ' || e.display, ' ')
                             FROM memory_entities me JOIN entities e ON e.entity_id=me.entity_id
                             WHERE me.memory_id=h.memory_id AND me.revision=h.head_revision), ''),
                   coalesce((SELECT group_concat(a.path || ' ' || a.symbol, ' ')
                             FROM memory_artifacts ma JOIN artifacts a ON a.artifact_id=ma.artifact_id
                             WHERE ma.memory_id=h.memory_id AND ma.revision=h.head_revision), '')
            FROM memory_heads h
            JOIN memory_revisions r
              ON r.memory_id=h.memory_id AND r.revision=h.head_revision;
            PRAGMA user_version=2;
            ",
        )
        .map_err(|error| Error::Migration(error.to_string()))?;
    Ok(())
}

fn migrate_v3(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            r"
            -- Workspace is an independent isolation boundary even for two
            -- workspaces sharing a repository/branch scope key. This remains
            -- non-unique so every database/snapshot valid under v1/v2,
            -- including explicit-ID canonical duplicates, still migrates.
            DROP INDEX memory_heads_canonical;
            CREATE INDEX memory_heads_canonical
                ON memory_heads(
                    namespace,
                    scope_key,
                    workspace_id,
                    kind,
                    canonical_key,
                    updated_seq DESC,
                    memory_id
                )
                WHERE canonical_key IS NOT NULL;

            -- New attachment rows use an opaque, stable scope partition in
            -- `namespace`; retrieval authorization comes from the joined
            -- memory head. Keep legacy plain-namespace rows searchable too.
            DROP INDEX entities_canonical;
            CREATE INDEX entities_canonical
                ON entities(canonical,entity_id);
            CREATE INDEX entities_display_folded
                ON entities(lower(display),entity_id);

            PRAGMA user_version=3;
            ",
        )
        .map_err(|error| Error::Migration(error.to_string()))?;
    Ok(())
}

fn migrate_v4(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            r"
            -- Scope remains canonical in scope_json. This expression index is
            -- a rebuildable projection used to ground session checkpoints
            -- without scanning every historical event.
            CREATE INDEX events_session_scope
                ON events(
                    namespace,
                    json_extract(scope_json,'$.workspace_id'),
                    json_extract(scope_json,'$.repository.repo_id'),
                    json_extract(scope_json,'$.repository.branch'),
                    json_extract(scope_json,'$.session_id'),
                    seq DESC
                )
                WHERE json_extract(scope_json,'$.session_id') IS NOT NULL;

            -- Head rows remain optimized for current reads. This companion
            -- table preserves every revision's ranking, validity, lifecycle,
            -- and canonical metadata instead of projecting history through
            -- the latest head.
            CREATE TABLE memory_revision_metadata (
                memory_id         TEXT NOT NULL,
                revision          INTEGER NOT NULL,
                kind              TEXT NOT NULL,
                state             TEXT NOT NULL,
                canonical_key     TEXT,
                importance        REAL NOT NULL,
                confidence        REAL NOT NULL,
                trust             TEXT NOT NULL,
                valid_from_ms     INTEGER,
                valid_until_ms    INTEGER,
                expires_at_ms     INTEGER,
                metadata_complete INTEGER NOT NULL CHECK(metadata_complete IN (0,1)),
                PRIMARY KEY(memory_id, revision),
                FOREIGN KEY(memory_id, revision)
                    REFERENCES memory_revisions(memory_id, revision)
            );
            INSERT INTO memory_revision_metadata(
                memory_id,revision,kind,state,canonical_key,importance,
                confidence,trust,valid_from_ms,valid_until_ms,expires_at_ms,
                metadata_complete
            )
            SELECT r.memory_id,r.revision,h.kind,h.state,h.canonical_key,
                   h.importance,h.confidence,h.trust,h.valid_from_ms,
                   h.valid_until_ms,h.expires_at_ms,
                   CASE WHEN r.revision=h.head_revision
                             AND r.recorded_seq=h.updated_seq
                        THEN 1 ELSE 0 END
            FROM memory_revisions r
            JOIN memory_heads h ON h.memory_id=r.memory_id;

            CREATE TABLE memory_link_revisions (
                link_id           TEXT NOT NULL,
                source_memory_id  TEXT NOT NULL,
                source_revision   INTEGER NOT NULL,
                target_memory_id  TEXT NOT NULL,
                relation          TEXT NOT NULL,
                weight            INTEGER NOT NULL CHECK(weight >= 0 AND weight <= 1000),
                created_event_id  TEXT NOT NULL REFERENCES events(event_id),
                created_at_ms     INTEGER NOT NULL,
                PRIMARY KEY(source_memory_id,source_revision,target_memory_id,relation),
                FOREIGN KEY(source_memory_id,source_revision)
                    REFERENCES memory_revisions(memory_id,revision),
                FOREIGN KEY(target_memory_id) REFERENCES memory_heads(memory_id)
            );
            INSERT INTO memory_link_revisions(
                link_id,source_memory_id,source_revision,target_memory_id,
                relation,weight,created_event_id,created_at_ms
            )
            SELECT l.link_id,l.source_memory_id,r.revision,
                   l.target_memory_id,l.relation,l.weight,
                   l.created_event_id,l.created_at_ms
            FROM memory_links l
            JOIN events e ON e.event_id=l.created_event_id
            JOIN memory_revisions r
              ON r.memory_id=l.source_memory_id AND r.recorded_seq=e.seq;
            PRAGMA user_version=4;
            ",
        )
        .map_err(|error| Error::Migration(error.to_string()))?;
    Ok(())
}

fn migrate_v5(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            r"
            -- Exact diagnostic lookup is a first-class retrieval channel. The
            -- partial expression index stays derived from canonical revision
            -- attributes and avoids scanning every current revision.
            CREATE INDEX memory_revisions_error_fingerprint
                ON memory_revisions(
                    json_extract(attributes_json,'$.error_fingerprint'),
                    memory_id,
                    revision
                )
                WHERE json_extract(attributes_json,'$.error_fingerprint') IS NOT NULL;

            -- Background enrichment and coverage reporting use exact durable
            -- scopes. Unlike the canonical-key index, this covers every live
            -- head and also satisfies pending-work ordering without sorting
            -- unrelated memories from the same namespace.
            CREATE INDEX memory_heads_search_scope
                ON memory_heads(
                    namespace,
                    scope_key,
                    workspace_id,
                    updated_seq,
                    memory_id
                )
                WHERE state!='retracted';

            -- Alias text lives in contentless FTS. This small derived marker
            -- makes missing, revised, or algorithm-outdated alias projections
            -- detectable without treating generated terms as snapshot truth.
            CREATE TABLE search_alias_state (
                memory_id          TEXT PRIMARY KEY,
                revision           INTEGER NOT NULL
                    CHECK(revision > 0),
                algorithm_version  INTEGER NOT NULL
                    CHECK(algorithm_version > 0),
                FOREIGN KEY(memory_id, revision)
                    REFERENCES memory_revisions(memory_id, revision)
                    ON DELETE CASCADE
            );

            -- Profiles describe immutable, reproducible search generators.
            -- They contain no provider secret or machine-local model path.
            CREATE TABLE search_profiles (
                profile_id         TEXT PRIMARY KEY
                    CHECK(length(profile_id) BETWEEN 1 AND 256),
                model_digest       TEXT NOT NULL
                    CHECK(length(model_digest) BETWEEN 1 AND 256),
                dimensions         INTEGER
                    CHECK(dimensions IS NULL OR
                          (typeof(dimensions)='integer' AND
                           dimensions BETWEEN 1 AND 4096)),
                signature_version  INTEGER NOT NULL DEFAULT 1
                    CHECK(typeof(signature_version)='integer' AND
                          signature_version=1),
                created_at_ms      INTEGER NOT NULL
                    CHECK(typeof(created_at_ms)='integer')
            );

            CREATE TRIGGER search_profiles_immutable
            BEFORE UPDATE ON search_profiles
            BEGIN
                SELECT RAISE(ABORT, 'search profiles are immutable');
            END;

            -- Search projections are disposable derivatives of one current
            -- canonical memory revision. Expansion-only profiles leave the
            -- vector triplet NULL. Dense profiles store exact little-endian
            -- f32 bytes, a deterministic 128-bit random-hyperplane signature,
            -- and a finite positive norm. A later revision replaces the row
            -- for the same profile and memory instead of accumulating stale
            -- vectors in the hot index.
            CREATE TABLE search_projections (
                profile_id         TEXT NOT NULL,
                memory_id          TEXT NOT NULL,
                revision           INTEGER NOT NULL
                    CHECK(revision > 0),
                content_hash       TEXT NOT NULL
                    CHECK(length(content_hash)=64 AND
                          content_hash NOT GLOB '*[^0-9a-f]*'),
                expansion          TEXT NOT NULL DEFAULT ''
                    CHECK(length(expansion) <= 262144),
                vector             BLOB,
                signature          BLOB,
                norm               REAL,
                indexed_at_ms      INTEGER NOT NULL
                    CHECK(typeof(indexed_at_ms)='integer'),
                PRIMARY KEY(profile_id, memory_id),
                FOREIGN KEY(profile_id) REFERENCES search_profiles(profile_id)
                    ON DELETE CASCADE,
                FOREIGN KEY(memory_id, revision)
                    REFERENCES memory_revisions(memory_id, revision)
                    ON DELETE CASCADE,
                CHECK(
                    (vector IS NULL AND signature IS NULL AND norm IS NULL) OR
                    (typeof(vector)='blob' AND length(vector) > 0 AND
                     typeof(signature)='blob' AND length(signature) > 0 AND
                     typeof(norm) IN ('real','integer') AND
                     norm > 0.0 AND norm < 1.0e308)
                )
            );
            CREATE INDEX search_projections_source
                ON search_projections(memory_id, revision, profile_id, content_hash);
            CREATE INDEX search_projections_profile_revision
                ON search_projections(profile_id, revision, content_hash, memory_id);

            -- Registration is a compare-and-swap against the current head.
            -- Running inference never holds a database lock; these triggers
            -- reject its result if the memory changed while it was in flight.
            CREATE TRIGGER search_projections_current_insert
            BEFORE INSERT ON search_projections
            WHEN NOT EXISTS (
                SELECT 1
                FROM memory_heads h
                JOIN memory_revisions r
                  ON r.memory_id=h.memory_id AND r.revision=h.head_revision
                WHERE h.memory_id=NEW.memory_id
                  AND h.head_revision=NEW.revision
                  AND r.content_hash=NEW.content_hash
                  AND h.state!='retracted'
            )
            BEGIN
                SELECT RAISE(ABORT, 'search projection source is not current');
            END;

            CREATE TRIGGER search_projections_current_update
            BEFORE UPDATE ON search_projections
            WHEN NOT EXISTS (
                SELECT 1
                FROM memory_heads h
                JOIN memory_revisions r
                  ON r.memory_id=h.memory_id AND r.revision=h.head_revision
                WHERE h.memory_id=NEW.memory_id
                  AND h.head_revision=NEW.revision
                  AND r.content_hash=NEW.content_hash
                  AND h.state!='retracted'
            )
            BEGIN
                SELECT RAISE(ABORT, 'search projection source is not current');
            END;

            -- SQLite CHECK constraints cannot reference another table. These
            -- companion triggers bind dense payload sizes to the immutable
            -- profile dimension on both insert and replacement.
            CREATE TRIGGER search_projections_vector_insert
            BEFORE INSERT ON search_projections
            WHEN NEW.vector IS NOT NULL AND NOT EXISTS (
                SELECT 1
                FROM search_profiles p
                WHERE p.profile_id=NEW.profile_id
                  AND p.dimensions IS NOT NULL
                  AND length(NEW.vector)=p.dimensions*4
                  AND length(NEW.signature)=16
            )
            BEGIN
                SELECT RAISE(ABORT, 'search projection vector or signature shape mismatch');
            END;

            CREATE TRIGGER search_projections_vector_update
            BEFORE UPDATE ON search_projections
            WHEN NEW.vector IS NOT NULL AND NOT EXISTS (
                SELECT 1
                FROM search_profiles p
                WHERE p.profile_id=NEW.profile_id
                  AND p.dimensions IS NOT NULL
                  AND length(NEW.vector)=p.dimensions*4
                  AND length(NEW.signature)=16
            )
            BEGIN
                SELECT RAISE(ABORT, 'search projection vector or signature shape mismatch');
            END;

            -- FTS remains a contentless, rebuildable projection. Aliases are
            -- deterministic code-oriented terms; expansions are generated off
            -- the critical path and never become canonical memory evidence.
            DROP TABLE memory_fts;
            CREATE VIRTUAL TABLE memory_fts USING fts5(
                title,
                body,
                tags,
                entities,
                paths,
                aliases,
                expansions,
                tokenize = 'unicode61 remove_diacritics 2',
                content = '',
                contentless_delete = 1
            );
            INSERT INTO memory_fts(
                rowid,title,body,tags,entities,paths,aliases,expansions
            )
            SELECT h.docid,
                   r.title,
                   r.body,
                   coalesce((SELECT group_concat(tag, ' ') FROM (
                       SELECT t.tag AS tag FROM memory_tags t
                       WHERE t.memory_id=h.memory_id
                         AND t.revision=h.head_revision
                       ORDER BY t.normalized,t.tag
                   )), ''),
                   coalesce((SELECT group_concat(entity, ' ') FROM (
                       SELECT e.canonical || ' ' || e.display AS entity
                       FROM memory_entities me
                       JOIN entities e ON e.entity_id=me.entity_id
                       WHERE me.memory_id=h.memory_id
                         AND me.revision=h.head_revision
                       ORDER BY e.kind,e.canonical,e.display,e.entity_id
                   )), ''),
                   coalesce((SELECT group_concat(path, ' ') FROM (
                       SELECT a.path || ' ' || a.symbol AS path
                       FROM memory_artifacts ma
                       JOIN artifacts a ON a.artifact_id=ma.artifact_id
                       WHERE ma.memory_id=h.memory_id
                         AND ma.revision=h.head_revision
                       ORDER BY a.repo_id,a.path,a.symbol,a.content_hash,
                                a.git_oid,a.language,a.artifact_id
                   )), ''),
                   '',
                   ''
            FROM memory_heads h
            JOIN memory_revisions r
              ON r.memory_id=h.memory_id AND r.revision=h.head_revision
            WHERE h.state!='retracted';

            PRAGMA user_version=5;
            ",
        )
        .map_err(|error| Error::Migration(error.to_string()))?;
    Ok(())
}

fn migrate_v6(connection: &Connection) -> Result<()> {
    connection
        .execute_batch(
            r"
            -- Generator identity remains immutable, while operators can
            -- disable a profile without deleting expensive derived data.
            CREATE TABLE search_profile_state (
                profile_id         TEXT PRIMARY KEY,
                active             INTEGER NOT NULL DEFAULT 1
                    CHECK(active IN (0,1)),
                FOREIGN KEY(profile_id) REFERENCES search_profiles(profile_id)
                    ON DELETE CASCADE
            );
            INSERT INTO search_profile_state(profile_id,active)
            SELECT profile_id,1 FROM search_profiles;
            CREATE TRIGGER search_profiles_create_state
            AFTER INSERT ON search_profiles
            BEGIN
                INSERT INTO search_profile_state(profile_id,active)
                VALUES(NEW.profile_id,1);
            END;

            -- Expansion text is indexed per immutable profile projection.
            -- Keeping one FTS row per projection prevents alphabetically
            -- earlier profiles from consuming a shared per-memory byte cap.
            CREATE VIRTUAL TABLE search_expansion_fts USING fts5(
                expansion,
                tokenize = 'unicode61 remove_diacritics 2',
                content = '',
                contentless_delete = 1
            );
            INSERT INTO search_expansion_fts(rowid,expansion)
            SELECT rowid,expansion
            FROM search_projections
            WHERE expansion!='';

            CREATE TRIGGER search_projections_expansion_insert
            AFTER INSERT ON search_projections
            WHEN NEW.expansion!=''
            BEGIN
                INSERT INTO search_expansion_fts(rowid,expansion)
                VALUES(NEW.rowid,NEW.expansion);
            END;
            CREATE TRIGGER search_projections_expansion_update
            AFTER UPDATE OF expansion ON search_projections
            BEGIN
                DELETE FROM search_expansion_fts WHERE rowid=OLD.rowid;
                INSERT INTO search_expansion_fts(rowid,expansion)
                SELECT NEW.rowid,NEW.expansion WHERE NEW.expansion!='';
            END;
            CREATE TRIGGER search_projections_expansion_delete
            BEFORE DELETE ON search_projections
            WHEN OLD.expansion!=''
            BEGIN
                DELETE FROM search_expansion_fts WHERE rowid=OLD.rowid;
            END;

            CREATE INDEX memory_link_revisions_target
                ON memory_link_revisions(
                    target_memory_id,
                    source_memory_id,
                    source_revision,
                    weight DESC,
                    relation
                );

            PRAGMA user_version=6;
            ",
        )
        .map_err(|error| Error::Migration(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use super::*;

    #[test]
    fn v1_migration_rebuilds_search_as_contentless_without_losing_matches() {
        let connection = Connection::open_in_memory().unwrap();
        migrate_v1(&connection).unwrap();
        connection
            .execute_batch(
                r#"
                INSERT INTO events(seq,event_id,namespace,kind,scope_json,content,attributes_json,trust,occurred_at_ms,ingested_at_ms,content_hash,redaction_count)
                VALUES
                    (1,'018f0000-0000-7000-8000-000000000001','default','explicit_memory','{"namespace":"default"}','source one','{}','explicit',1,1,'hash-one',0),
                    (2,'018f0000-0000-7000-8000-000000000005','default','explicit_memory','{"namespace":"default"}','source two','{}','explicit',2,2,'hash-two',0);
                INSERT INTO memory_heads(docid,memory_id,namespace,scope_key,kind,state,head_revision,importance,confidence,trust,created_at_ms,updated_at_ms,created_seq,updated_seq)
                VALUES
                    (1,'018f0000-0000-7000-8000-000000000002','default','scope','procedure','active',2,0.9,0.8,'user_confirmed',1,2,1,2),
                    (2,'018f0000-0000-7000-8000-000000000003','default','scope','fact','retracted',1,0.5,0.5,'explicit',1,2,1,2);
                INSERT INTO memory_revisions(memory_id,revision,title,body,attributes_json,scope_json,content_hash,recorded_at_ms,recorded_seq)
                VALUES
                    ('018f0000-0000-7000-8000-000000000002',1,'Migration test old','older searchable needle','{}','{"namespace":"default"}','hash-old',1,1),
                    ('018f0000-0000-7000-8000-000000000002',2,'Migration test','contentless searchable needle','{}','{"namespace":"default"}','hash-new',2,2),
                    ('018f0000-0000-7000-8000-000000000003',1,'Migration target','link target','{}','{"namespace":"default"}','target-hash',1,1);
                INSERT INTO memory_links(link_id,source_memory_id,target_memory_id,relation,weight,created_event_id,created_at_ms)
                VALUES('018f0000-0000-7000-8000-000000000004','018f0000-0000-7000-8000-000000000002','018f0000-0000-7000-8000-000000000003','documents',400,'018f0000-0000-7000-8000-000000000001',1);
                INSERT INTO memory_fts(rowid,title,body,tags,entities,paths)
                VALUES(1,'Migration test','contentless searchable needle','','','');
                "#,
            )
            .unwrap();

        initialize(&connection, &EngineOptions::default()).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
                .unwrap(),
            SCHEMA_VERSION
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'needle'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        let metadata_fidelity = connection
            .prepare(
                "SELECT revision,metadata_complete FROM memory_revision_metadata WHERE memory_id='018f0000-0000-7000-8000-000000000002' ORDER BY revision",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, bool>(1)?)))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(metadata_fidelity, [(1, false), (2, true)]);
        assert!(!connection
            .query_row(
                "SELECT metadata_complete FROM memory_revision_metadata WHERE memory_id='018f0000-0000-7000-8000-000000000003' AND revision=1",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
        assert_eq!(
            connection
                .query_row(
                    "SELECT source_revision FROM memory_link_revisions WHERE link_id='018f0000-0000-7000-8000-000000000004'",
                    [],
                    |row| row.get::<_, u32>(0),
                )
                .unwrap(),
            1
        );
        assert!(
            connection
                .query_row("SELECT body FROM memory_fts WHERE rowid=1", [], |row| {
                    row.get::<_, Option<String>>(0)
                })
                .unwrap()
                .is_none(),
            "contentless FTS must not duplicate canonical memory bodies"
        );
        connection
            .execute("DELETE FROM memory_fts WHERE rowid=1", [])
            .unwrap();
        connection
            .execute(
                "INSERT INTO memory_fts(rowid,title,body,tags,entities,paths) VALUES(1,'Migration test','contentless searchable needle','','','')",
                [],
            )
            .unwrap();

        for (sql, expected_index) in [
            (
                "EXPLAIN QUERY PLAN SELECT me.memory_id FROM entities e JOIN memory_entities me ON me.entity_id=e.entity_id WHERE e.canonical='component'",
                "entities_canonical",
            ),
            (
                "EXPLAIN QUERY PLAN SELECT me.memory_id FROM entities e JOIN memory_entities me ON me.entity_id=e.entity_id WHERE lower(e.display)='component'",
                "entities_display_folded",
            ),
            (
                "EXPLAIN QUERY PLAN SELECT ma.memory_id FROM artifacts a JOIN memory_artifacts ma ON ma.artifact_id=a.artifact_id WHERE a.namespace='default' AND a.repo_id='repo' AND a.path='src/lib.rs'",
                "memory_artifacts_memory_lookup",
            ),
        ] {
            let mut statement = connection.prepare(sql).unwrap();
            let plan = statement
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap();
            assert!(
                plan.iter().any(|detail| detail.contains(expected_index)),
                "expected {expected_index} in query plan: {plan:?}"
            );
        }

        let canonical_columns = connection
            .prepare("PRAGMA index_info(memory_heads_canonical)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(2))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            canonical_columns,
            [
                "namespace",
                "scope_key",
                "workspace_id",
                "kind",
                "canonical_key",
                "updated_seq",
                "memory_id"
            ]
        );
    }

    #[test]
    fn v4_database_upgrades_through_search_profile_lifecycle() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        migrate_v1(&connection).unwrap();
        migrate_v2(&connection).unwrap();
        migrate_v3(&connection).unwrap();
        migrate_v4(&connection).unwrap();
        connection
            .execute_batch(
                r#"
                INSERT INTO events(
                    seq,event_id,namespace,kind,scope_json,content,
                    attributes_json,trust,occurred_at_ms,ingested_at_ms,
                    content_hash,redaction_count
                ) VALUES(
                    1,'018f0000-0000-7000-8000-000000000101','default',
                    'explicit_memory','{"namespace":"default"}','source',
                    '{}','explicit',1,1,
                    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',0
                );
                INSERT INTO memory_heads(
                    docid,memory_id,namespace,scope_key,kind,state,
                    head_revision,importance,confidence,trust,created_at_ms,
                    updated_at_ms,created_seq,updated_seq
                ) VALUES(
                    1,'018f0000-0000-7000-8000-000000000102','default',
                    'scope','procedure','active',1,0.8,0.9,'explicit',1,1,1,1
                );
                INSERT INTO memory_revisions(
                    memory_id,revision,title,body,attributes_json,scope_json,
                    content_hash,recorded_at_ms,recorded_seq
                ) VALUES(
                    '018f0000-0000-7000-8000-000000000102',1,
                    'Dense migration','semantic searchable needle',
                    '{"error_fingerprint":"rustc:E0277"}',
                    '{"namespace":"default"}',
                    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                    1,1
                );
                INSERT INTO memory_fts(rowid,title,body,tags,entities,paths)
                VALUES(1,'Dense migration','semantic searchable needle','','','');
                "#,
            )
            .unwrap();

        migrate_v5(&connection).unwrap();
        migrate_v6(&connection).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
                .unwrap(),
            SCHEMA_VERSION
        );
        let fts_columns = connection
            .prepare("PRAGMA table_info(memory_fts)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            fts_columns,
            [
                "title",
                "body",
                "tags",
                "entities",
                "paths",
                "aliases",
                "expansions"
            ]
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'needle'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        assert!(connection
            .query_row(
                "SELECT 1 FROM sqlite_schema WHERE type='table' AND name='search_expansion_fts'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .is_ok());
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM search_alias_state", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0,
            "legacy heads must be detected as needing alias backfill"
        );
        assert!(connection
            .query_row(
                "SELECT 1 FROM sqlite_schema WHERE type='index' AND name='memory_revisions_error_fingerprint'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .is_ok());
        let fingerprint_plan = connection
            .prepare(
                "EXPLAIN QUERY PLAN SELECT memory_id FROM memory_revisions WHERE json_extract(attributes_json,'$.error_fingerprint')=?1",
            )
            .unwrap()
            .query_map(["rustc:E0277"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        assert!(fingerprint_plan.contains("memory_revisions_error_fingerprint"));
        let pending_plan = connection
            .prepare(
                "EXPLAIN QUERY PLAN SELECT memory_id FROM memory_heads WHERE namespace=?1 AND scope_key=?2 AND workspace_id IS ?3 AND state!='retracted' ORDER BY updated_seq,memory_id LIMIT 10",
            )
            .unwrap()
            .query_map(
                rusqlite::params!["default", "scope", Option::<String>::None],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
            .join("\n");
        assert!(pending_plan.contains("memory_heads_search_scope"));
        let expansion_plan = connection
            .prepare(
                "EXPLAIN QUERY PLAN SELECT expansion FROM search_projections WHERE memory_id=?1 AND revision=?2 AND expansion!='' ORDER BY profile_id",
            )
            .unwrap()
            .query_map(
                rusqlite::params!["018f0000-0000-7000-8000-000000000102", 1],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            expansion_plan
                .iter()
                .any(|detail| detail.contains("search_projections_source")),
            "expected source index in query plan: {expansion_plan:?}"
        );
        assert!(
            expansion_plan
                .iter()
                .all(|detail| !detail.contains("USE TEMP B-TREE")),
            "source lookup should stream profile order: {expansion_plan:?}"
        );

        connection
            .execute(
                "INSERT INTO search_alias_state(memory_id,revision,algorithm_version) VALUES(?1,1,1)",
                ["018f0000-0000-7000-8000-000000000102"],
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    "UPDATE search_alias_state SET revision=2 WHERE memory_id=?1",
                    ["018f0000-0000-7000-8000-000000000102"],
                )
                .is_err()
        );

        connection
            .execute_batch(
                r"
                INSERT INTO search_profiles(profile_id,model_digest,dimensions,created_at_ms)
                VALUES('dense-v1','digest-v1',3,10);
                INSERT INTO search_profiles(profile_id,model_digest,dimensions,created_at_ms)
                VALUES('expansion-v1','digest-v1',NULL,10);
                INSERT INTO search_projections(
                    profile_id,memory_id,revision,content_hash,expansion,
                    vector,signature,norm,indexed_at_ms
                ) VALUES(
                    'dense-v1','018f0000-0000-7000-8000-000000000102',1,
                    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                    'likely semantic query',zeroblob(12),zeroblob(16),1.0,11
                );
                INSERT INTO search_projections(
                    profile_id,memory_id,revision,content_hash,expansion,
                    vector,signature,norm,indexed_at_ms
                ) VALUES(
                    'expansion-v1','018f0000-0000-7000-8000-000000000102',1,
                    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                    'another likely query',NULL,NULL,NULL,11
                );
                ",
            )
            .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM search_profile_state WHERE active=1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM search_expansion_fts WHERE search_expansion_fts MATCH 'another'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        connection
            .execute(
                "DELETE FROM search_profiles WHERE profile_id='expansion-v1'",
                [],
            )
            .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM search_expansion_fts WHERE search_expansion_fts MATCH 'another'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        assert!(
            connection
                .execute(
                    "UPDATE search_profiles SET model_digest='changed' WHERE profile_id='dense-v1'",
                    [],
                )
                .is_err()
        );
        assert!(
            connection
                .execute(
                    "UPDATE search_projections SET vector=zeroblob(8) WHERE profile_id='dense-v1'",
                    [],
                )
                .is_err()
        );
        assert!(connection
            .execute(
                "UPDATE search_projections SET signature=zeroblob(1) WHERE profile_id='dense-v1'",
                [],
            )
            .is_err());
        assert!(
            connection
                .execute(
                    "UPDATE search_projections SET content_hash=?1 WHERE profile_id='dense-v1'",
                    ["cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"],
                )
                .is_err()
        );
        assert!(connection
            .execute(
                "INSERT INTO search_profiles(profile_id,model_digest,dimensions,created_at_ms) VALUES('bad-dim','digest',0,12)",
                [],
            )
            .is_err());
        assert!(connection
            .execute(
                "INSERT INTO search_profiles(profile_id,model_digest,dimensions,signature_version,created_at_ms) VALUES('future-signature','digest',3,2,12)",
                [],
            )
            .is_err());
        assert!(
            connection
                .execute(
                    "UPDATE search_projections SET signature=NULL WHERE profile_id='dense-v1'",
                    [],
                )
                .is_err()
        );
    }

    #[test]
    fn concurrent_first_open_serializes_schema_migrations() {
        let directory = tempfile::tempdir().unwrap();
        for round in 0..8 {
            let database = directory.path().join(format!("memory-{round}.sqlite3"));
            let workers = 8;
            let barrier = Arc::new(Barrier::new(workers));
            let handles = (0..workers)
                .map(|_| {
                    let database = database.clone();
                    let barrier = Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        let connection = Connection::open(database).unwrap();
                        barrier.wait();
                        initialize(&connection, &EngineOptions::default())
                    })
                })
                .collect::<Vec<_>>();

            for handle in handles {
                handle
                    .join()
                    .unwrap_or_else(|_| panic!("first-open worker panicked in round {round}"))
                    .unwrap_or_else(|error| {
                        panic!("first-open worker failed in round {round}: {error}")
                    });
            }
            let connection = Connection::open(database).unwrap();
            assert_eq!(
                connection
                    .query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
                    .unwrap(),
                SCHEMA_VERSION
            );
            assert_eq!(
                connection
                    .query_row("PRAGMA application_id", [], |row| row.get::<_, u32>(0))
                    .unwrap(),
                APPLICATION_ID
            );
            assert_eq!(
                connection
                    .query_row(
                        "SELECT count(*) FROM sqlite_schema WHERE name='memory_heads'",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap(),
                1
            );
        }
    }
}
