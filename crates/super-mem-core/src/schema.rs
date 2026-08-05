//! `SQLite` schema and connection initialization.

use std::{
    fs::File,
    io::{ErrorKind, Read},
    path::Path,
};

use rusqlite::Connection;

use crate::{Durability, EngineOptions, Error, Result};

pub(crate) const SCHEMA_VERSION: u32 = 4;
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
    let Ok(mut file) = File::open(path) else {
        return Ok(false);
    };
    // Read immutable database-header fields directly. Opening SQLite here can
    // trigger rollback-journal recovery or reject an otherwise identifiable
    // store whose crash journal is damaged, exactly when purge is most useful.
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
    connection.busy_timeout(std::time::Duration::from_millis(options.busy_timeout_ms))?;
    connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;")?;

    // Verify identity before enabling WAL, which would otherwise mutate an
    // unrelated SQLite file supplied accidentally. The same check is repeated
    // after acquiring the migration writer lock because another process may
    // initialize an empty file between these two steps.
    validate_identity(connection)?;

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

    connection.execute_batch("BEGIN IMMEDIATE;")?;
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

fn validate_identity(connection: &Connection) -> Result<u32> {
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
    fn concurrent_first_open_serializes_schema_migrations() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("memory.sqlite3");
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
            handle.join().unwrap().unwrap();
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
