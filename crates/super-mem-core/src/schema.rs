//! `SQLite` schema and connection initialization.

use std::{
    fs,
    fs::File,
    io::{ErrorKind, Read, Seek, SeekFrom},
    path::Path,
    thread,
    time::{Duration, Instant},
};

#[cfg(not(windows))]
use std::io::Write;

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, params};

use crate::{
    DatabaseDiagnostics, DatabaseInspection, Durability, EngineOptions, Error, Result,
    applicability::artifact_fingerprint,
};

pub(crate) const SCHEMA_VERSION: u32 = 6;
const MAX_INSPECTION_SQLITE_VALUE_BYTES: i32 = 4 * 1_024 * 1_024;
const MAX_INSPECTION_SQL_TEXT_BYTES: i32 = 1_024 * 1_024;
const INSPECTION_WORK_DEADLINE: Duration = Duration::from_secs(5);
#[cfg(windows)]
const MAX_WINDOWS_INSPECTION_SNAPSHOT_BYTES: u64 = 512 * 1_024 * 1_024;
/// `SQLite` application identifier (`SMEM`) used to distinguish stores from
/// unrelated files before destructive maintenance operations.
pub const APPLICATION_ID: u32 = 0x534D_454D;

/// Opaque identity captured from the same metadata snapshot a caller audited.
///
/// Passing this identity back to [`inspect_database_at_identity`] joins an
/// outer file-security snapshot to the descriptor-pinned `SQLite` inspection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseFileIdentity(String);

impl DatabaseFileIdentity {
    /// Stable, machine-local diagnostic label for correlating before/after
    /// observations without exposing raw device or file-index fields.
    #[must_use]
    pub fn diagnostic_digest(&self) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"super-mem doctor file identity v1\0");
        hasher.update(self.0.as_bytes());
        hasher.finalize().to_hex().to_string()
    }
}

/// Captures the bounded file identity used to join a diagnostic snapshot to
/// [`inspect_database_at_identity`].
pub fn database_file_identity(
    path: impl AsRef<Path>,
    metadata: &fs::Metadata,
) -> Result<DatabaseFileIdentity> {
    platform_database_file_identity(path.as_ref(), metadata)
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
fn platform_database_file_identity(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<DatabaseFileIdentity> {
    use std::os::unix::fs::MetadataExt;
    Ok(DatabaseFileIdentity(format!(
        "unix:{}:{}:{}:{}:{}:{}:{}",
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.mode(),
        metadata.nlink()
    )))
}

#[cfg(windows)]
fn platform_database_file_identity(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<DatabaseFileIdentity> {
    windows_database_file_identity(path, metadata)
}

#[cfg(not(any(unix, windows)))]
#[allow(clippy::unnecessary_wraps)]
fn platform_database_file_identity(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<DatabaseFileIdentity> {
    Ok(DatabaseFileIdentity(format!(
        "fallback:{}:{:?}",
        metadata.len(),
        metadata.modified().ok()
    )))
}

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

/// Inspects an initialized database without creating it or running migrations.
///
/// When no WAL or rollback journal is present, the source is identity-pinned
/// and protected with native `SQLite`-compatible locks. A stable copy is then
/// streamed into a private temporary file on Unix or a bounded `SQLite`-owned
/// allocation on Windows, and only that copy is opened by `SQLite`. The source
/// and its sidecars are never opened by `SQLite`, created, recovered,
/// checkpointed, or migrated.
pub fn inspect_database(
    path: impl AsRef<Path>,
    writer_timeout_ms: u64,
) -> Result<DatabaseInspection> {
    let path = path.as_ref();
    let expected = fs::symlink_metadata(path).map_err(Error::Io)?;
    let expected_identity = database_file_identity(path, &expected)?;
    inspect_database_at_identity(path, writer_timeout_ms, &expected_identity)
}

/// Inspects a database only if it is still the exact file captured by an
/// earlier caller-owned metadata snapshot.
///
/// This closes the gap between a CLI file-security preflight and the pinned
/// `SQLite` descriptor used for inspection. A path exchange at either boundary
/// fails closed instead of inspecting a different valid store.
pub fn inspect_database_at_identity(
    path: impl AsRef<Path>,
    writer_timeout_ms: u64,
    expected_identity: &DatabaseFileIdentity,
) -> Result<DatabaseInspection> {
    const MAX_FINDINGS: usize = 32;
    const MAX_FINDING_BYTES: usize = 1_024;

    let path = path.as_ref();
    if writer_timeout_ms == 0 {
        return Err(Error::InvalidInput(
            "database inspection writer timeout must be positive".into(),
        ));
    }
    let identity_before = fs::symlink_metadata(path).map_err(Error::Io)?;
    if database_file_identity(path, &identity_before)? != *expected_identity {
        return Err(Error::InvalidInput(
            "database identity changed after the caller's preflight snapshot".into(),
        ));
    }
    if !identity_before.is_file() || identity_before.file_type().is_symlink() {
        return Err(Error::InvalidInput(format!(
            "database does not exist or is not a regular file: {}",
            path.display()
        )));
    }
    let inspection_deadline = Instant::now() + INSPECTION_WORK_DEADLINE;
    let pinned = pin_database(path, &identity_before, expected_identity)?;
    if let Some(reason) = pinned.recovery_sidecar_blocker()? {
        return Err(Error::Migration(reason));
    }
    let header_uses_wal = pinned.header_uses_wal()?;
    let writer_probe = pinned.acquire_snapshot_guard(header_uses_wal)?;
    #[cfg(not(windows))]
    let snapshot = pinned.private_snapshot(inspection_deadline)?;
    if let Some(reason) = pinned.recovery_sidecar_blocker()? {
        return Err(Error::Migration(reason));
    }

    #[cfg(not(windows))]
    let connection = {
        let read_flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        let snapshot_uri = immutable_sqlite_uri(snapshot.path())?;
        Connection::open_with_flags(&snapshot_uri, read_flags)?
    };
    #[cfg(windows)]
    let connection = {
        let length = pinned.database.metadata().map_err(Error::Io)?.len();
        if length > MAX_WINDOWS_INSPECTION_SNAPSHOT_BYTES {
            return Err(Error::InvalidInput(format!(
                "database exceeds the {}-byte in-memory Windows inspection bound",
                MAX_WINDOWS_INSPECTION_SNAPSHOT_BYTES
            )));
        }
        let length = usize::try_from(length).map_err(|_| {
            Error::InvalidInput("database is too large for this Windows process".into())
        })?;
        let source = pinned.database.try_clone().map_err(Error::Io)?;
        let snapshot =
            read_windows_inspection_snapshot(source, length, inspection_deadline, header_uses_wal)?;
        let mut connection = Connection::open_in_memory()?;
        connection.deserialize(rusqlite::MAIN_DB, snapshot, true)?;
        if Instant::now() >= inspection_deadline {
            return Err(Error::Migration(
                "database snapshot exceeded the five-second inspection deadline".into(),
            ));
        }
        connection
    };
    configure_inspection_connection(&connection, inspection_deadline)?;
    connection.busy_timeout(Duration::from_millis(writer_timeout_ms))?;
    connection.execute_batch("BEGIN;")?;
    let schema_version = validate_identity(&connection)?;
    if schema_version == 0 {
        return Err(Error::Migration(
            "database is not an initialized super-mem store".into(),
        ));
    }

    let mut statement = connection.prepare("PRAGMA quick_check(33)")?;
    let mut rows = statement.query([])?;
    let mut quick_check_findings = Vec::new();
    let mut quick_check_ok = true;
    let mut quick_check_total = 0_usize;
    while let Some(row) = rows.next()? {
        let finding = row.get::<_, String>(0)?;
        if finding == "ok" {
            continue;
        }
        quick_check_ok = false;
        quick_check_total = quick_check_total.saturating_add(1);
        if quick_check_findings.len() < MAX_FINDINGS {
            quick_check_findings.push(truncate_diagnostic(&finding, MAX_FINDING_BYTES));
        }
    }
    let quick_check_truncated = quick_check_total > MAX_FINDINGS;
    drop(rows);
    drop(statement);

    let mut statement = connection.prepare("SELECT 1 FROM pragma_foreign_key_check LIMIT 33")?;
    let mut rows = statement.query([])?;
    let mut foreign_key_violations = 0_u64;
    while rows.next()?.is_some() {
        foreign_key_violations = foreign_key_violations.saturating_add(1);
    }
    let foreign_key_violations_truncated = foreign_key_violations > MAX_FINDINGS as u64;
    foreign_key_violations = foreign_key_violations.min(MAX_FINDINGS as u64);
    drop(rows);
    drop(statement);
    let (schema_manifest_ok, schema_manifest_findings, schema_manifest_truncated) =
        inspect_schema_manifest(&connection, schema_version, MAX_FINDINGS)?;
    let (
        application_invariants_ok,
        application_invariant_findings,
        application_invariant_findings_truncated,
    ) = inspect_application_invariants(&connection, schema_version, MAX_FINDINGS)?;
    let count = |sql: &str| -> Result<u64> {
        let value: i64 = connection.query_row(sql, [], |row| row.get(0))?;
        Ok(value.max(0) as u64)
    };
    let database_seq = connection
        .query_row("SELECT coalesce(max(seq),0) FROM events", [], |row| {
            row.get::<_, i64>(0)
        })?
        .max(0);
    let page_count: i64 = connection.query_row("PRAGMA page_count", [], |row| row.get(0))?;
    let page_size: i64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    let events = count("SELECT count(*) FROM events")?;
    let active_memories =
        count("SELECT count(*) FROM memory_heads WHERE state IN ('active','contested')")?;
    let superseded_memories = count("SELECT count(*) FROM memory_heads WHERE state='superseded'")?;
    let retracted_memories = count("SELECT count(*) FROM memory_heads WHERE state='retracted'")?;
    connection.execute_batch("ROLLBACK;")?;
    drop(connection);

    let identity_after_read = pinned.metadata()?;
    if !same_file_identity(&identity_before, &identity_after_read) {
        return Err(Error::InvalidInput(
            "database path identity changed during read-only inspection".into(),
        ));
    }

    let (writer_lock_checked, writer_lock_available, writer_lock_error) =
        writer_probe.diagnostics(MAX_FINDING_BYTES);
    let identity_after_probe = pinned.metadata()?;
    if !same_file_identity(&identity_before, &identity_after_probe) {
        return Err(Error::InvalidInput(
            "database path identity changed during writer-lock probe".into(),
        ));
    }
    let healthy = quick_check_ok
        && foreign_key_violations == 0
        && schema_manifest_ok
        && application_invariants_ok
        && schema_version == SCHEMA_VERSION
        && writer_lock_checked
        && writer_lock_available;
    Ok(DatabaseInspection {
        schema_version,
        database_seq,
        events,
        active_memories,
        superseded_memories,
        retracted_memories,
        database_bytes: page_count.max(0).saturating_mul(page_size.max(0)) as u64,
        diagnostics: DatabaseDiagnostics {
            quick_check_ok,
            quick_check_findings,
            quick_check_truncated,
            foreign_key_violations,
            foreign_key_violations_truncated,
            schema_manifest_ok,
            schema_current: schema_version == SCHEMA_VERSION,
            schema_manifest_findings,
            schema_manifest_truncated,
            application_invariants_ok,
            application_invariant_findings,
            application_invariant_findings_truncated,
            writer_lock_checked,
            writer_lock_available,
            writer_lock_error,
            healthy,
        },
    })
}

#[cfg(any(windows, test))]
struct WindowsInspectionSnapshotReader {
    source: File,
    offset: usize,
    deadline: Instant,
    normalize_wal_header: bool,
}

#[cfg(any(windows, test))]
#[allow(unsafe_code)]
fn read_windows_inspection_snapshot(
    mut source: File,
    length: usize,
    deadline: Instant,
    normalize_wal_header: bool,
) -> Result<rusqlite::serialize::OwnedData> {
    source.seek(SeekFrom::Start(0)).map_err(Error::Io)?;
    let mut reader = WindowsInspectionSnapshotReader {
        source,
        offset: 0,
        deadline,
        normalize_wal_header,
    };
    let allocation_bytes = u64::try_from(length).map_err(|_| {
        Error::InvalidInput("database is too large for SQLite's snapshot allocator".into())
    })?;
    let pointer = unsafe { rusqlite::ffi::sqlite3_malloc64(allocation_bytes) }.cast::<u8>();
    let pointer = std::ptr::NonNull::new(pointer).ok_or_else(|| {
        Error::Migration(format!(
            "SQLite could not allocate the {length}-byte Windows inspection snapshot"
        ))
    })?;
    // SAFETY: `pointer` came from `sqlite3_malloc64`; ownership is transferred
    // before any fallible read, so every error or panic frees the allocation.
    let snapshot = unsafe { rusqlite::serialize::OwnedData::from_raw_nonnull(pointer, length) };
    // SAFETY: the allocation is exactly `length` bytes and remains exclusively
    // owned by `snapshot` for the duration of this initialization.
    let output = unsafe { std::slice::from_raw_parts_mut(pointer.as_ptr(), length) };
    reader.read_exact(output).map_err(Error::Io)?;
    if Instant::now() >= deadline {
        return Err(Error::Migration(
            "database snapshot exceeded the five-second inspection deadline".into(),
        ));
    }
    Ok(snapshot)
}

#[cfg(any(windows, test))]
impl Read for WindowsInspectionSnapshotReader {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if Instant::now() >= self.deadline {
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "database snapshot exceeded the five-second inspection deadline",
            ));
        }
        let bounded = output.len().min(64 * 1_024);
        let read = self.source.read(&mut output[..bounded])?;
        if self.normalize_wal_header {
            for header_offset in [18_usize, 19] {
                if (self.offset..self.offset.saturating_add(read)).contains(&header_offset) {
                    output[header_offset - self.offset] = 1;
                }
            }
        }
        self.offset = self.offset.saturating_add(read);
        Ok(read)
    }
}

fn configure_inspection_connection(connection: &Connection, deadline: Instant) -> Result<()> {
    connection.set_limit(
        rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,
        MAX_INSPECTION_SQLITE_VALUE_BYTES,
    )?;
    connection.set_limit(
        rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH,
        MAX_INSPECTION_SQL_TEXT_BYTES,
    )?;
    connection.progress_handler(1_000, Some(move || Instant::now() >= deadline))?;
    Ok(())
}

pub(crate) fn inspect_schema_manifest(
    connection: &Connection,
    schema_version: u32,
    maximum_findings: usize,
) -> Result<(bool, Vec<String>, bool)> {
    const MAX_SCHEMA_OBJECTS: usize = 512;
    const MAX_SCHEMA_TYPE_BYTES: i64 = 64;
    const MAX_SCHEMA_NAME_BYTES: i64 = 1_024;
    const MAX_SCHEMA_SQL_BYTES: i64 = 64 * 1_024;
    let expected = Connection::open_in_memory()?;
    migrate_v1(&expected)?;
    if schema_version >= 2 {
        migrate_v2(&expected)?;
    }
    if schema_version >= 3 {
        migrate_v3(&expected)?;
    }
    if schema_version >= 4 {
        migrate_v4(&expected)?;
    }
    if schema_version >= 5 {
        migrate_v5(&expected)?;
    }
    if schema_version >= 6 {
        migrate_v6(&expected)?;
    }

    let mut statement = expected.prepare(
        "SELECT type,name,tbl_name,sql FROM sqlite_schema
         ORDER BY type,name",
    )?;
    let mut expected_objects = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        })?
        .map(|object| {
            object.map(|(object_type, name, table, sql)| ((object_type, name), (table, sql)))
        })
        .collect::<std::result::Result<std::collections::BTreeMap<_, _>, _>>()?;
    let mut actual_statement = connection.prepare(
        "SELECT length(CAST(type AS BLOB)),substr(CAST(type AS BLOB),1,65),
                length(CAST(name AS BLOB)),substr(CAST(name AS BLOB),1,1025),
                length(CAST(tbl_name AS BLOB)),substr(CAST(tbl_name AS BLOB),1,1025),
                length(CAST(sql AS BLOB)),substr(CAST(sql AS BLOB),1,65537)
         FROM sqlite_schema LIMIT 513",
    )?;
    let mut actual_objects = actual_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Vec<u8>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<Vec<u8>>>(7)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    actual_objects.sort_by(|left, right| (&left.1, &left.3).cmp(&(&right.1, &right.3)));
    let mut findings = Vec::new();
    let mut total_findings = 0_usize;
    let mut record = |finding: String| {
        total_findings = total_findings.saturating_add(1);
        if findings.len() < maximum_findings {
            findings.push(finding);
        }
    };
    if actual_objects.len() > MAX_SCHEMA_OBJECTS {
        record(format!("more than {MAX_SCHEMA_OBJECTS} schema objects"));
        actual_objects.truncate(MAX_SCHEMA_OBJECTS);
    }
    for (
        object_type_length,
        object_type_bytes,
        name_length,
        name_bytes,
        table_length,
        table_bytes,
        actual_sql_length,
        actual_sql,
    ) in actual_objects
    {
        if object_type_length > MAX_SCHEMA_TYPE_BYTES {
            record("oversized schema object type".into());
            continue;
        }
        let object_type = String::from_utf8(object_type_bytes)
            .map_err(|_| Error::Migration("schema object type is not valid UTF-8".into()))?;
        if name_length > MAX_SCHEMA_NAME_BYTES || table_length > MAX_SCHEMA_NAME_BYTES {
            record(format!("oversized {object_type} schema identifier"));
            continue;
        }
        let name = String::from_utf8(name_bytes)
            .map_err(|_| Error::Migration("schema object name is not valid UTF-8".into()))?;
        let table = String::from_utf8(table_bytes)
            .map_err(|_| Error::Migration("schema table name is not valid UTF-8".into()))?;
        let key = (object_type.clone(), name.clone());
        let Some((expected_table, expected_sql)) = expected_objects.remove(&key) else {
            record(format!("unexpected {object_type} {name}"));
            continue;
        };
        let matches = table == expected_table
            && match (expected_sql.as_deref(), actual_sql_length, actual_sql) {
                (None, None, None) => true,
                (Some(expected_sql), Some(length), Some(actual_sql))
                    if length <= MAX_SCHEMA_SQL_BYTES =>
                {
                    String::from_utf8(actual_sql).is_ok_and(|actual_sql| {
                        normalize_schema_sql(&actual_sql) == normalize_schema_sql(expected_sql)
                    })
                }
                _ => false,
            };
        if !matches {
            record(format!("changed {object_type} {name}"));
        }
    }
    for ((object_type, name), _) in expected_objects {
        record(format!("missing {object_type} {name}"));
    }
    Ok((
        total_findings == 0,
        findings,
        total_findings > maximum_findings,
    ))
}

pub(crate) fn inspect_application_invariants(
    connection: &Connection,
    schema_version: u32,
    maximum_findings: usize,
) -> Result<(bool, Vec<String>, bool)> {
    let mut findings = Vec::new();
    let mut total_findings = 0_usize;
    let mut check = |name: &str, sql: &str| -> Result<()> {
        if connection
            .query_row(sql, [], |row| row.get::<_, i64>(0))
            .optional()?
            .is_some()
        {
            total_findings = total_findings.saturating_add(1);
            if findings.len() < maximum_findings {
                findings.push(name.to_owned());
            }
        }
        Ok(())
    };
    check(
        "memory_head_without_head_revision",
        "SELECT 1
         FROM memory_heads h
         WHERE NOT EXISTS (
             SELECT 1 FROM memory_revisions r
             WHERE r.memory_id=h.memory_id AND r.revision=h.head_revision
         )
         LIMIT 1",
    )?;
    check(
        "memory_head_revision_not_latest",
        "SELECT 1
         FROM memory_heads h
         WHERE EXISTS (
             SELECT 1 FROM memory_revisions r
             WHERE r.memory_id=h.memory_id AND r.revision>h.head_revision
         )
         LIMIT 1",
    )?;
    if schema_version >= 4 {
        check(
            "memory_revision_without_metadata",
            "SELECT 1
             FROM memory_revisions r
             WHERE NOT EXISTS (
                 SELECT 1 FROM memory_revision_metadata m
                 WHERE m.memory_id=r.memory_id AND m.revision=r.revision
             )
             LIMIT 1",
        )?;
    }
    Ok((
        total_findings == 0,
        findings,
        total_findings > maximum_findings,
    ))
}

fn normalize_schema_sql(value: &str) -> String {
    value.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(not(unix))]
fn recovery_sidecar_blocker(path: &Path) -> Option<String> {
    let wal = sidecar_path(path, "-wal");
    let shm = sidecar_path(path, "-shm");
    let journal = sidecar_path(path, "-journal");
    if journal.exists() {
        return Some(
            "database has a rollback journal; recovery is required before inspection".into(),
        );
    }
    let wal_present = wal.exists();
    let shm_present = shm.exists();
    if wal_present != shm_present {
        return Some(
            "database has an incomplete WAL sidecar set; recovery is required before inspection"
                .into(),
        );
    }
    wal_present.then(|| {
        "database has live WAL state; observational inspection is unavailable until it is checkpointed"
            .into()
    })
}

#[cfg(any(not(unix), test))]
fn sidecar_path(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push(suffix);
    value.into()
}

fn immutable_sqlite_uri(path: &Path) -> Result<std::path::PathBuf> {
    let path = path.to_str().ok_or_else(|| {
        Error::InvalidInput(format!(
            "private SQLite snapshot path is not valid UTF-8: {}",
            path.display()
        ))
    })?;
    let mut uri = String::with_capacity(path.len().saturating_mul(3).saturating_add(18));
    uri.push_str("file:");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            uri.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(uri, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    uri.push_str("?immutable=1");
    Ok(uri.into())
}

#[cfg(not(windows))]
fn private_database_snapshot(
    database: &File,
    deadline: Instant,
) -> Result<tempfile::NamedTempFile> {
    let before = database.metadata().map_err(Error::Io)?;
    let mut source = database.try_clone().map_err(Error::Io)?;
    source.seek(SeekFrom::Start(0)).map_err(Error::Io)?;
    let mut snapshot = tempfile::Builder::new()
        .prefix("super-mem-doctor-")
        .tempfile()
        .map_err(Error::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        snapshot
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(Error::Io)?;
    }
    let mut copied = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1_024];
    loop {
        if Instant::now() >= deadline {
            return Err(Error::Migration(
                "database snapshot exceeded the five-second inspection deadline".into(),
            ));
        }
        let read = source.read(&mut buffer).map_err(Error::Io)?;
        if read == 0 {
            break;
        }
        snapshot.write_all(&buffer[..read]).map_err(Error::Io)?;
        copied = copied.saturating_add(read as u64);
        if copied > before.len() {
            return Err(Error::InvalidInput(
                "database grew while creating the private inspection snapshot".into(),
            ));
        }
    }
    snapshot.flush().map_err(Error::Io)?;
    if copied != before.len() {
        return Err(Error::InvalidInput(format!(
            "database length changed while creating the private inspection snapshot: expected {} bytes, copied {copied}",
            before.len()
        )));
    }
    let after = database.metadata().map_err(Error::Io)?;
    if !same_file_identity(&before, &after) {
        return Err(Error::InvalidInput(
            "database identity changed while creating the private inspection snapshot".into(),
        ));
    }
    Ok(snapshot)
}

struct SnapshotGuard {
    #[cfg(any(unix, windows))]
    file: File,
    #[cfg(any(unix, windows))]
    shared_locked: bool,
    #[cfg(any(unix, windows))]
    reserved_locked: bool,
    available: bool,
    error: Option<String>,
}

impl SnapshotGuard {
    fn diagnostics(&self, maximum_error_bytes: usize) -> (bool, bool, Option<String>) {
        (
            true,
            self.available,
            self.error
                .as_deref()
                .map(|error| truncate_diagnostic(error, maximum_error_bytes)),
        )
    }
}

#[cfg(unix)]
struct PinnedDatabase {
    directory: File,
    database: File,
    database_name: std::ffi::OsString,
}

#[cfg(unix)]
impl PinnedDatabase {
    fn recovery_sidecar_blocker(&self) -> Result<Option<String>> {
        unix_recovery_sidecar_blocker(&self.directory, &self.database_name)
    }

    fn metadata(&self) -> Result<fs::Metadata> {
        self.database.metadata().map_err(Error::Io)
    }

    fn header_uses_wal(&self) -> Result<bool> {
        header_uses_wal(&self.database)
    }

    fn acquire_snapshot_guard(&self, header_uses_wal: bool) -> Result<SnapshotGuard> {
        acquire_unix_snapshot_guard(self, header_uses_wal)
    }

    fn private_snapshot(&self, deadline: Instant) -> Result<tempfile::NamedTempFile> {
        private_database_snapshot(&self.database, deadline)
    }
}

#[cfg(unix)]
fn pin_database(
    path: &Path,
    expected: &fs::Metadata,
    _expected_identity: &DatabaseFileIdentity,
) -> Result<PinnedDatabase> {
    use std::os::unix::fs::OpenOptionsExt;

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(Error::Io)?.join(path)
    };
    reject_linked_parent_components(&absolute)?;
    let parent = absolute.parent().ok_or_else(|| {
        Error::InvalidInput(format!(
            "database path has no parent: {}",
            absolute.display()
        ))
    })?;
    let expected_parent = fs::symlink_metadata(parent).map_err(Error::Io)?;
    if !expected_parent.is_dir() || expected_parent.file_type().is_symlink() {
        return Err(Error::InvalidInput(format!(
            "database parent is not a real directory: {}",
            parent.display()
        )));
    }
    let mut directory_options = fs::OpenOptions::new();
    directory_options
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let directory = directory_options.open(parent).map_err(Error::Io)?;
    if !same_file_identity(&expected_parent, &directory.metadata().map_err(Error::Io)?) {
        return Err(Error::InvalidInput(
            "database parent identity changed while it was pinned".into(),
        ));
    }

    let name = absolute.file_name().ok_or_else(|| {
        Error::InvalidInput(format!(
            "database path has no file name: {}",
            absolute.display()
        ))
    })?;
    let database = open_unix_database_at(&directory, name, false)?;
    let opened = database.metadata().map_err(Error::Io)?;
    if !opened.is_file() || !same_file_identity(expected, &opened) {
        return Err(Error::InvalidInput(
            "database identity changed while it was pinned".into(),
        ));
    }
    Ok(PinnedDatabase {
        directory,
        database,
        database_name: name.to_os_string(),
    })
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn open_unix_database_at(directory: &File, name: &std::ffi::OsStr, write: bool) -> Result<File> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::ffi::OsStrExt,
    };

    let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| {
        Error::InvalidInput("database file name contains an embedded NUL byte".into())
    })?;
    let access = if write { libc::O_RDWR } else { libc::O_RDONLY };
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            access | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn unix_recovery_sidecar_blocker(
    directory: &File,
    database_name: &std::ffi::OsStr,
) -> Result<Option<String>> {
    let journal_present = unix_relative_entry_exists(directory, database_name, "-journal")?;
    if journal_present {
        return Ok(Some(
            "database has a rollback journal; recovery is required before inspection".into(),
        ));
    }
    let wal_present = unix_relative_entry_exists(directory, database_name, "-wal")?;
    let shm_present = unix_relative_entry_exists(directory, database_name, "-shm")?;
    if wal_present != shm_present {
        return Ok(Some(
            "database has an incomplete WAL sidecar set; recovery is required before inspection"
                .into(),
        ));
    }
    Ok(wal_present.then(|| {
        "database has live WAL state; observational inspection is unavailable until it is checkpointed"
            .into()
    }))
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn unix_relative_entry_exists(
    directory: &File,
    database_name: &std::ffi::OsStr,
    suffix: &str,
) -> Result<bool> {
    use std::os::{fd::AsRawFd, unix::ffi::OsStrExt};

    let mut name = database_name.to_os_string();
    name.push(suffix);
    let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| {
        Error::InvalidInput("database sidecar name contains an embedded NUL byte".into())
    })?;
    let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            directory.as_raw_fd(),
            name.as_ptr(),
            metadata.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ENOENT) {
        Ok(false)
    } else {
        Err(Error::Io(error))
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn acquire_unix_snapshot_guard(
    pinned: &PinnedDatabase,
    header_uses_wal: bool,
) -> Result<SnapshotGuard> {
    use std::os::unix::io::AsRawFd;

    let parent_writable = unix_effective_parent_writable(&pinned.directory)?;
    let (file, main_writable, mut error) =
        match open_unix_database_at(&pinned.directory, &pinned.database_name, true) {
            Ok(file) => (file, true, None),
            Err(open_error) => (
                pinned.database.try_clone().map_err(Error::Io)?,
                false,
                Some(format!(
                    "database is not writable through its pinned file descriptor: {open_error}"
                )),
            ),
        };
    if !same_file_identity(
        &pinned.database.metadata().map_err(Error::Io)?,
        &file.metadata().map_err(Error::Io)?,
    ) {
        return Err(Error::InvalidInput(
            "database identity changed while acquiring the inspection lock".into(),
        ));
    }
    acquire_unix_shared_snapshot_lock(file.as_raw_fd())?;
    if !parent_writable {
        error.get_or_insert_with(|| {
            "database parent does not grant effective write and search access".into()
        });
    }
    let reserved_locked = if header_uses_wal || !main_writable || !parent_writable {
        false
    } else {
        unix_set_lock(
            file.as_raw_fd(),
            libc::F_WRLCK as libc::c_int,
            SQLITE_RESERVED_BYTE,
            1,
        )?
    };
    if !header_uses_wal && main_writable && parent_writable && !reserved_locked {
        error.get_or_insert_with(|| "SQLite reserved writer lock is already held".into());
    }
    let available = main_writable && parent_writable && (header_uses_wal || reserved_locked);
    Ok(SnapshotGuard {
        file,
        shared_locked: true,
        reserved_locked,
        available,
        error,
    })
}

#[cfg(any(unix, windows))]
const SQLITE_PENDING_BYTE: i64 = 0x4000_0000;
#[cfg(any(unix, windows))]
const SQLITE_RESERVED_BYTE: i64 = SQLITE_PENDING_BYTE + 1;
#[cfg(any(unix, windows))]
const SQLITE_SHARED_FIRST: i64 = SQLITE_PENDING_BYTE + 2;
#[cfg(any(unix, windows))]
const SQLITE_SHARED_SIZE: i64 = 510;

#[cfg(unix)]
#[allow(unsafe_code)]
fn unix_effective_parent_writable(directory: &File) -> Result<bool> {
    use std::os::fd::AsRawFd;
    let result = unsafe {
        libc::faccessat(
            directory.as_raw_fd(),
            c".".as_ptr(),
            libc::W_OK | libc::X_OK,
            libc::AT_EACCESS,
        )
    };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(libc::EACCES | libc::EPERM)) {
        Ok(false)
    } else {
        Err(Error::Io(error))
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn unix_set_lock(
    fd: std::os::fd::RawFd,
    lock_type: libc::c_int,
    start: i64,
    len: i64,
) -> Result<bool> {
    let mut lock = libc::flock {
        l_type: lock_type as _,
        l_whence: libc::SEEK_SET as _,
        l_start: start as _,
        l_len: len as _,
        l_pid: 0,
    };
    let command = unix_open_description_lock_command()?;
    let result = unsafe { libc::fcntl(fd, command, &raw mut lock) };
    if result == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if matches!(error.raw_os_error(), Some(libc::EACCES | libc::EAGAIN)) {
        Ok(false)
    } else {
        Err(Error::Io(error))
    }
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
fn unix_open_description_lock_command() -> Result<libc::c_int> {
    #[cfg(any(target_os = "linux", target_os = "android", target_os = "macos"))]
    {
        Ok(libc::F_OFD_SETLK)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
    {
        Err(Error::Migration(
            "observational inspection requires open-file-description locks on this Unix platform"
                .into(),
        ))
    }
}

#[cfg(unix)]
fn acquire_unix_shared_snapshot_lock(fd: std::os::fd::RawFd) -> Result<()> {
    let pending_locked = unix_set_lock(fd, libc::F_RDLCK as libc::c_int, SQLITE_PENDING_BYTE, 1)?;
    if !pending_locked {
        return Err(Error::Migration(
            "database has a pending writer; a stable inspection snapshot cannot be created".into(),
        ));
    }

    let shared = unix_set_lock(
        fd,
        libc::F_RDLCK as libc::c_int,
        SQLITE_SHARED_FIRST,
        SQLITE_SHARED_SIZE,
    );
    match shared {
        Ok(true) => {}
        Ok(false) => {
            let _ = unix_set_lock(fd, libc::F_UNLCK as libc::c_int, SQLITE_PENDING_BYTE, 1);
            return Err(Error::Migration(
                "database has an exclusive lock; a stable inspection snapshot cannot be created"
                    .into(),
            ));
        }
        Err(error) => {
            let _ = unix_set_lock(fd, libc::F_UNLCK as libc::c_int, SQLITE_PENDING_BYTE, 1);
            return Err(error);
        }
    }

    match unix_set_lock(fd, libc::F_UNLCK as libc::c_int, SQLITE_PENDING_BYTE, 1) {
        Ok(true) => Ok(()),
        Ok(false) => {
            let _ = unix_set_lock(
                fd,
                libc::F_UNLCK as libc::c_int,
                SQLITE_SHARED_FIRST,
                SQLITE_SHARED_SIZE,
            );
            Err(Error::Migration(
                "database pending-byte lock could not be released after snapshot acquisition"
                    .into(),
            ))
        }
        Err(error) => {
            let _ = unix_set_lock(
                fd,
                libc::F_UNLCK as libc::c_int,
                SQLITE_SHARED_FIRST,
                SQLITE_SHARED_SIZE,
            );
            Err(error)
        }
    }
}

#[cfg(unix)]
impl Drop for SnapshotGuard {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        if self.reserved_locked {
            let _ = unix_set_lock(
                self.file.as_raw_fd(),
                libc::F_UNLCK as libc::c_int,
                SQLITE_RESERVED_BYTE,
                1,
            );
        }
        if self.shared_locked {
            let _ = unix_set_lock(
                self.file.as_raw_fd(),
                libc::F_UNLCK as libc::c_int,
                SQLITE_SHARED_FIRST,
                SQLITE_SHARED_SIZE,
            );
        }
    }
}

#[cfg(unix)]
fn reject_linked_parent_components(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(Error::Io)?.join(path)
    };
    let parent = absolute.parent().ok_or_else(|| {
        Error::InvalidInput(format!("database path has no parent: {}", path.display()))
    })?;
    let mut cursor = std::path::PathBuf::new();
    for component in parent.components() {
        cursor.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&cursor).map_err(Error::Io)?;
        if metadata.file_type().is_symlink() && !trusted_system_path_alias(&cursor, &metadata) {
            return Err(Error::InvalidInput(format!(
                "database parent contains a symbolic link: {}",
                cursor.display()
            )));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn trusted_system_path_alias(path: &Path, metadata: &fs::Metadata) -> bool {
    if !metadata.file_type().is_symlink() {
        return false;
    }
    let expected = if path == Path::new("/var") {
        Path::new("/private/var")
    } else if path == Path::new("/tmp") {
        Path::new("/private/tmp")
    } else {
        return false;
    };
    fs::read_link(path).is_ok_and(|target| {
        let target = if target.is_absolute() {
            target
        } else {
            Path::new("/").join(target)
        };
        target == expected
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn trusted_system_path_alias(_path: &Path, _metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
struct PinnedDatabase {
    _directory: File,
    database: File,
    identity: DatabaseFileIdentity,
    sidecar_base: std::path::PathBuf,
}

#[cfg(windows)]
impl PinnedDatabase {
    fn recovery_sidecar_blocker(&self) -> Result<Option<String>> {
        Ok(recovery_sidecar_blocker(&self.sidecar_base))
    }

    fn metadata(&self) -> Result<fs::Metadata> {
        self.database.metadata().map_err(Error::Io)
    }

    fn header_uses_wal(&self) -> Result<bool> {
        header_uses_wal(&self.database)
    }

    fn acquire_snapshot_guard(&self, _header_uses_wal: bool) -> Result<SnapshotGuard> {
        acquire_windows_snapshot_guard(self)
    }
}

#[cfg(windows)]
fn pin_database(
    path: &Path,
    expected: &fs::Metadata,
    expected_identity: &DatabaseFileIdentity,
) -> Result<PinnedDatabase> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().map_err(Error::Io)?.join(path)
    };
    let parent = absolute.parent().ok_or_else(|| {
        Error::InvalidInput(format!(
            "database path has no parent: {}",
            absolute.display()
        ))
    })?;
    let directory = open_windows_path(parent, false, true)?;
    let database = open_windows_path(&absolute, false, false)?;
    let opened = database.metadata().map_err(Error::Io)?;
    if !windows_metadata_shape_matches(expected, &opened) {
        return Err(Error::InvalidInput(
            "database metadata changed while its Windows handle was pinned".into(),
        ));
    }
    let identity = windows_database_file_identity_from_file(&database)?;
    if &identity != expected_identity {
        return Err(Error::InvalidInput(
            "database file ID changed while its Windows handle was pinned".into(),
        ));
    }
    Ok(PinnedDatabase {
        _directory: directory,
        database,
        identity,
        sidecar_base: absolute,
    })
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn acquire_windows_snapshot_guard(pinned: &PinnedDatabase) -> Result<SnapshotGuard> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
    };

    let (file, main_writable, mut error) =
        match open_windows_path(&pinned.sidecar_base, true, false) {
            Ok(file) => (file, true, None),
            Err(open_error) => (
                open_windows_path(&pinned.sidecar_base, false, false)?,
                false,
                Some(format!(
                    "database is not writable through its pinned Windows handle: {open_error}"
                )),
            ),
        };
    if windows_database_file_identity_from_file(&file)? != pinned.identity {
        return Err(Error::InvalidInput(
            "database file ID changed while acquiring its Windows writer lock".into(),
        ));
    }
    let parent_writable = match open_windows_path(
        pinned.sidecar_base.parent().ok_or_else(|| {
            Error::InvalidInput("database path has no Windows parent directory".into())
        })?,
        true,
        true,
    ) {
        Ok(_) => true,
        Err(parent_error) => {
            error.get_or_insert_with(|| {
                format!("database parent does not grant Windows create access: {parent_error}")
            });
            false
        }
    };
    let mut pending = windows_overlapped(SQLITE_PENDING_BYTE as u64);
    let pending_locked = unsafe {
        LockFileEx(
            file.as_raw_handle().cast(),
            LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &raw mut pending,
        )
    };
    if pending_locked == 0 {
        return Err(Error::Migration(format!(
            "database pending snapshot lock is unavailable: {}",
            std::io::Error::last_os_error()
        )));
    }
    let mut shared = windows_overlapped(SQLITE_SHARED_FIRST as u64);
    let shared_locked = unsafe {
        LockFileEx(
            file.as_raw_handle().cast(),
            LOCKFILE_FAIL_IMMEDIATELY,
            0,
            SQLITE_SHARED_SIZE as u32,
            0,
            &raw mut shared,
        )
    };
    if shared_locked == 0 {
        let shared_error = std::io::Error::last_os_error();
        use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
        let _ = unsafe { UnlockFileEx(file.as_raw_handle().cast(), 0, 1, 0, &raw mut pending) };
        return Err(Error::Migration(format!(
            "database shared snapshot lock is unavailable: {shared_error}"
        )));
    }
    use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
    let pending_released =
        unsafe { UnlockFileEx(file.as_raw_handle().cast(), 0, 1, 0, &raw mut pending) };
    if pending_released == 0 {
        let release_error = std::io::Error::last_os_error();
        let _ = unsafe {
            UnlockFileEx(
                file.as_raw_handle().cast(),
                0,
                SQLITE_SHARED_SIZE as u32,
                0,
                &raw mut shared,
            )
        };
        return Err(Error::Io(release_error));
    }
    let reserved_locked = if main_writable && parent_writable {
        let mut reserved = windows_overlapped(SQLITE_RESERVED_BYTE as u64);
        let locked = unsafe {
            LockFileEx(
                file.as_raw_handle().cast(),
                LOCKFILE_FAIL_IMMEDIATELY | LOCKFILE_EXCLUSIVE_LOCK,
                0,
                1,
                0,
                &raw mut reserved,
            )
        };
        if locked == 0 {
            error.get_or_insert_with(|| {
                format!(
                    "database reserved writer lock is unavailable: {}",
                    std::io::Error::last_os_error()
                )
            });
            false
        } else {
            true
        }
    } else {
        false
    };
    Ok(SnapshotGuard {
        file,
        shared_locked: true,
        reserved_locked,
        available: main_writable && parent_writable && reserved_locked,
        error,
    })
}

#[cfg(windows)]
#[allow(unsafe_code)]
impl Drop for SnapshotGuard {
    fn drop(&mut self) {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
        if self.reserved_locked {
            let mut reserved = windows_overlapped(SQLITE_RESERVED_BYTE as u64);
            let _ = unsafe {
                UnlockFileEx(self.file.as_raw_handle().cast(), 0, 1, 0, &raw mut reserved)
            };
        }
        if self.shared_locked {
            let mut shared = windows_overlapped(SQLITE_SHARED_FIRST as u64);
            let _ = unsafe {
                UnlockFileEx(
                    self.file.as_raw_handle().cast(),
                    0,
                    SQLITE_SHARED_SIZE as u32,
                    0,
                    &raw mut shared,
                )
            };
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn windows_overlapped(offset: u64) -> windows_sys::Win32::System::IO::OVERLAPPED {
    use windows_sys::Win32::System::IO::OVERLAPPED;
    let mut overlapped = unsafe { std::mem::zeroed::<OVERLAPPED>() };
    overlapped.Anonymous.Anonymous.Offset = offset as u32;
    overlapped.Anonymous.Anonymous.OffsetHigh = (offset >> 32) as u32;
    overlapped
}

#[cfg(windows)]
fn open_windows_path(path: &Path, write: bool, directory: bool) -> Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };

    let mut options = fs::OpenOptions::new();
    options.read(true).write(write).custom_flags(
        FILE_FLAG_OPEN_REPARSE_POINT
            | if directory {
                FILE_FLAG_BACKUP_SEMANTICS
            } else {
                0
            },
    );
    let file = options.open(path).map_err(Error::Io)?;
    use std::os::windows::fs::MetadataExt;
    if file.metadata().map_err(Error::Io)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(Error::InvalidInput(format!(
            "refusing a Windows reparse point in the database path: {}",
            path.display()
        )));
    }
    Ok(file)
}

#[cfg(windows)]
fn windows_database_file_identity(
    path: &Path,
    expected: &fs::Metadata,
) -> Result<DatabaseFileIdentity> {
    let file = open_windows_path(path, false, false)?;
    let opened = file.metadata().map_err(Error::Io)?;
    if !windows_metadata_shape_matches(expected, &opened) {
        return Err(Error::InvalidInput(
            "database metadata changed while capturing its Windows file ID".into(),
        ));
    }
    windows_database_file_identity_from_file(&file)
}

#[cfg(windows)]
fn windows_metadata_shape_matches(first: &fs::Metadata, second: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    first.file_attributes() == second.file_attributes()
        && first.file_size() == second.file_size()
        && first.creation_time() == second.creation_time()
        && first.last_write_time() == second.last_write_time()
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn windows_database_file_identity_from_file(file: &File) -> Result<DatabaseFileIdentity> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO, FileIdInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    let succeeded = unsafe {
        GetFileInformationByHandle(file.as_raw_handle().cast(), information.as_mut_ptr())
    };
    if succeeded == 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    let information = unsafe { information.assume_init() };
    let mut identity = MaybeUninit::<FILE_ID_INFO>::uninit();
    let identity_succeeded = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle().cast(),
            FileIdInfo,
            identity.as_mut_ptr().cast(),
            u32::try_from(std::mem::size_of::<FILE_ID_INFO>())
                .expect("FILE_ID_INFO fits in a Windows API length"),
        )
    };
    if identity_succeeded == 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    let identity = unsafe { identity.assume_init() };
    let file_size =
        (u64::from(information.nFileSizeHigh) << 32) | u64::from(information.nFileSizeLow);
    let last_write = (u64::from(information.ftLastWriteTime.dwHighDateTime) << 32)
        | u64::from(information.ftLastWriteTime.dwLowDateTime);
    Ok(DatabaseFileIdentity(format!(
        "windows:{}:{:02x?}:{file_size}:{last_write}:{}:{}",
        identity.VolumeSerialNumber,
        identity.FileId.Identifier,
        information.dwFileAttributes,
        information.nNumberOfLinks
    )))
}

#[cfg(not(any(unix, windows)))]
struct PinnedDatabase {
    path: std::path::PathBuf,
    database: File,
}

#[cfg(not(any(unix, windows)))]
impl PinnedDatabase {
    fn recovery_sidecar_blocker(&self) -> Result<Option<String>> {
        Ok(recovery_sidecar_blocker(&self.path))
    }

    fn metadata(&self) -> Result<fs::Metadata> {
        self.database.metadata().map_err(Error::Io)
    }

    fn header_uses_wal(&self) -> Result<bool> {
        header_uses_wal(&self.database)
    }

    fn acquire_snapshot_guard(&self, _header_uses_wal: bool) -> Result<SnapshotGuard> {
        Ok(SnapshotGuard {
            available: false,
            error: Some("writer availability cannot be verified on this platform".into()),
        })
    }

    fn private_snapshot(&self, deadline: Instant) -> Result<tempfile::NamedTempFile> {
        private_database_snapshot(&self.database, deadline)
    }
}

#[cfg(not(any(unix, windows)))]
fn pin_database(
    path: &Path,
    _expected: &fs::Metadata,
    _expected_identity: &DatabaseFileIdentity,
) -> Result<PinnedDatabase> {
    Ok(PinnedDatabase {
        path: path.to_path_buf(),
        database: File::open(path).map_err(Error::Io)?,
    })
}

fn header_uses_wal(file: &File) -> Result<bool> {
    let mut file = file.try_clone().map_err(Error::Io)?;
    file.seek(SeekFrom::Start(18)).map_err(Error::Io)?;
    let mut versions = [0_u8; 2];
    file.read_exact(&mut versions).map_err(Error::Io)?;
    match versions {
        [2, 2] => Ok(true),
        [1, 1] => Ok(false),
        _ => Err(Error::Migration(format!(
            "unsupported SQLite header read/write versions: {}/{}",
            versions[0], versions[1]
        ))),
    }
}

#[cfg(unix)]
fn same_file_identity(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    before.dev() == after.dev()
        && before.ino() == after.ino()
        && before.len() == after.len()
        && before.mtime() == after.mtime()
        && before.mtime_nsec() == after.mtime_nsec()
        && before.mode() == after.mode()
        && before.nlink() == after.nlink()
}

#[cfg(not(unix))]
fn same_file_identity(before: &fs::Metadata, after: &fs::Metadata) -> bool {
    before.len() == after.len()
        && before.modified().ok() == after.modified().ok()
        && before.file_type() == after.file_type()
}

fn truncate_diagnostic(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.to_owned();
    }
    let mut end = maximum;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
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

            -- Historical checkpoint retries reconstruct whether an outcome
            -- was retracted at their original sequence boundary without
            -- scanning unrelated event associations.
            CREATE INDEX event_memories_memory
                ON event_memories(memory_id,event_id);

            -- Historical checkpoint retries start from the bounded automatic
            -- command keys, then join the exact revision at their sequence
            -- boundary. Keep this separate from the current-head canonical
            -- index used by ordinary checkpoints.
            CREATE INDEX memory_revision_metadata_checkpoint
                ON memory_revision_metadata(
                    kind,
                    canonical_key,
                    memory_id,
                    revision
                )
                WHERE canonical_key IS NOT NULL;

            -- Canonical artifact metadata can contain multi-KiB paths,
            -- symbols, and hashes. This rebuildable fixed-width projection
            -- lets recall stage and deduplicate applicability material without
            -- repeatedly transferring or sorting those strings.
            CREATE TABLE artifact_fingerprints (
                artifact_id       INTEGER PRIMARY KEY
                    REFERENCES artifacts(artifact_id) ON DELETE CASCADE,
                identity          BLOB CHECK(identity IS NULL OR length(identity)=32),
                content           BLOB CHECK(content IS NULL OR length(content)=32),
                CHECK((identity IS NULL)=(content IS NULL))
            );
            CREATE INDEX artifact_fingerprints_identity
                ON artifact_fingerprints(identity,artifact_id);
            ",
        )
        .map_err(|error| Error::Migration(error.to_string()))?;
    rebuild_artifact_fingerprints(connection)?;
    connection
        .execute_batch("PRAGMA user_version=6;")
        .map_err(|error| Error::Migration(error.to_string()))?;
    Ok(())
}

/// Rebuilds fixed-width artifact applicability fingerprints from canonical rows.
///
/// Rows are read and hashed one at a time so migration and snapshot restore do
/// not retain or sort a corpus of attacker-controlled paths and symbols.
pub(crate) fn rebuild_artifact_fingerprints(connection: &Connection) -> Result<()> {
    connection.execute("DELETE FROM artifact_fingerprints", [])?;
    let mut artifacts = connection.prepare(
        "SELECT artifact_id,repo_id,path,symbol,content_hash FROM artifacts ORDER BY artifact_id",
    )?;
    let mut insert = connection.prepare(
        "INSERT INTO artifact_fingerprints(artifact_id,identity,content) VALUES(?1,?2,?3)",
    )?;
    let mut rows = artifacts.query([])?;
    while let Some(row) = rows.next()? {
        let artifact_id = row.get::<_, i64>(0)?;
        let repo_id = row.get::<_, String>(1)?;
        let path = row.get::<_, String>(2)?;
        let symbol = row.get::<_, String>(3)?;
        let content_hash = row.get::<_, String>(4)?;
        if content_hash.is_empty() {
            insert.execute(params![
                artifact_id,
                Option::<&[u8]>::None,
                Option::<&[u8]>::None
            ])?;
        } else {
            let (identity, content) = artifact_fingerprint(
                &repo_id,
                &path,
                (!symbol.is_empty()).then_some(symbol.as_str()),
                &content_hash,
            )
            .digests();
            insert.execute(params![artifact_id, &identity[..], &content[..]])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        time::{Duration, Instant},
    };

    use super::*;

    fn initialized_file(path: &Path) {
        let connection = Connection::open(path).unwrap();
        initialize(&connection, &EngineOptions::default()).unwrap();
    }

    #[test]
    fn inspection_is_noncreating_nonmigrating_and_observational() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.sqlite3");
        assert!(inspect_database(&missing, 50).is_err());
        assert!(!missing.exists());

        let v1 = directory.path().join("v1.sqlite3");
        let connection = Connection::open(&v1).unwrap();
        migrate_v1(&connection).unwrap();
        connection
            .execute_batch("PRAGMA journal_mode=DELETE;")
            .unwrap();
        drop(connection);
        let before = fs::read(&v1).unwrap();
        let inspection = inspect_database(&v1, 50).unwrap();
        assert_eq!(inspection.schema_version, 1);
        assert!(inspection.diagnostics.schema_manifest_ok);
        assert!(!inspection.diagnostics.schema_current);
        assert!(!inspection.diagnostics.healthy);
        assert_eq!(fs::read(&v1).unwrap(), before);
        assert_eq!(u32::from_be_bytes(before[60..64].try_into().unwrap()), 1);

        let current = std::env::current_dir().unwrap();
        let relative_directory = tempfile::tempdir_in(&current).unwrap();
        let relative_database = relative_directory.path().join("relative.sqlite3");
        initialized_file(&relative_database);
        let relative_database = relative_database.strip_prefix(&current).unwrap();
        let inspection = inspect_database(relative_database, 50).unwrap();
        assert_eq!(inspection.schema_version, SCHEMA_VERSION);
        assert!(inspection.diagnostics.healthy);
    }

    #[test]
    fn inspection_bounds_writer_contention_without_normal_initialization() {
        if let (Some(database), Some(ready)) = (
            std::env::var_os("SUPER_MEM_INSPECTION_LOCK_DATABASE"),
            std::env::var_os("SUPER_MEM_INSPECTION_LOCK_READY"),
        ) {
            let connection = Connection::open(database).unwrap();
            connection.execute_batch("BEGIN IMMEDIATE;").unwrap();
            fs::write(ready, b"ready").unwrap();
            thread::sleep(Duration::from_secs(10));
            return;
        }

        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("locked.sqlite3");
        initialized_file(&database);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch("PRAGMA journal_mode=DELETE;")
            .unwrap();
        drop(connection);
        let ready = directory.path().join("writer-ready");
        let mut writer = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "schema::tests::inspection_bounds_writer_contention_without_normal_initialization",
                "--nocapture",
            ])
            .env("SUPER_MEM_INSPECTION_LOCK_DATABASE", &database)
            .env("SUPER_MEM_INSPECTION_LOCK_READY", &ready)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !ready.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(ready.exists(), "writer helper did not acquire its lock");
        let started = Instant::now();
        let inspection = inspect_database(&database, 50).unwrap();
        assert!(inspection.diagnostics.writer_lock_checked);
        assert!(!inspection.diagnostics.writer_lock_available);
        assert!(!inspection.diagnostics.healthy);
        assert!(started.elapsed() < Duration::from_secs(1));
        writer.kill().unwrap();
        writer.wait().unwrap();
    }

    #[cfg(any(unix, windows))]
    #[test]
    #[allow(unsafe_code)]
    fn inspection_refuses_a_pending_writer_before_joining_the_shared_range() {
        if let (Some(database), Some(ready)) = (
            std::env::var_os("SUPER_MEM_INSPECTION_PENDING_DATABASE"),
            std::env::var_os("SUPER_MEM_INSPECTION_PENDING_READY"),
        ) {
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(database)
                .unwrap();
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                assert!(
                    unix_set_lock(
                        file.as_raw_fd(),
                        libc::F_WRLCK as libc::c_int,
                        SQLITE_PENDING_BYTE,
                        1,
                    )
                    .unwrap()
                );
            }
            #[cfg(windows)]
            {
                use std::os::windows::io::AsRawHandle;
                use windows_sys::Win32::Storage::FileSystem::{
                    LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
                };
                let mut pending = windows_overlapped(SQLITE_PENDING_BYTE as u64);
                assert_ne!(
                    unsafe {
                        LockFileEx(
                            file.as_raw_handle().cast(),
                            LOCKFILE_FAIL_IMMEDIATELY | LOCKFILE_EXCLUSIVE_LOCK,
                            0,
                            1,
                            0,
                            &raw mut pending,
                        )
                    },
                    0
                );
            }
            fs::write(ready, b"ready").unwrap();
            thread::sleep(Duration::from_secs(10));
            return;
        }

        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("pending.sqlite3");
        initialized_file(&database);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
            .unwrap();
        drop(connection);
        let ready = directory.path().join("pending-ready");
        let mut writer = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "schema::tests::inspection_refuses_a_pending_writer_before_joining_the_shared_range",
                "--nocapture",
            ])
            .env("SUPER_MEM_INSPECTION_PENDING_DATABASE", &database)
            .env("SUPER_MEM_INSPECTION_PENDING_READY", &ready)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while !ready.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            ready.exists(),
            "pending-writer helper did not acquire its lock"
        );

        let started = Instant::now();
        let error = inspect_database(&database, 50).unwrap_err().to_string();
        assert!(error.contains("pending"), "unexpected error: {error}");
        assert!(started.elapsed() < Duration::from_secs(1));

        writer.kill().unwrap();
        writer.wait().unwrap();
        assert!(inspect_database(&database, 50).unwrap().diagnostics.healthy);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_private_snapshot_does_not_release_the_guard_lock() {
        if let (Some(database), Some(outcome_path)) = (
            std::env::var_os("SUPER_MEM_INSPECTION_OFD_DATABASE"),
            std::env::var_os("SUPER_MEM_INSPECTION_OFD_OUTCOME"),
        ) {
            let connection = Connection::open(database).unwrap();
            connection.busy_timeout(Duration::ZERO).unwrap();
            let result = connection.execute_batch("BEGIN IMMEDIATE; ROLLBACK;");
            let outcome_value = match result {
                Ok(()) => "acquired",
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if matches!(
                        error.code,
                        ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked
                    ) =>
                {
                    "blocked"
                }
                Err(error) => panic!("unexpected writer result: {error}"),
            };
            fs::write(outcome_path, outcome_value).unwrap();
            return;
        }

        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("ofd-lock.sqlite3");
        initialized_file(&database);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
            .unwrap();
        drop(connection);
        let metadata = fs::symlink_metadata(&database).unwrap();
        let identity = database_file_identity(&database, &metadata).unwrap();
        let pinned = pin_database(&database, &metadata, &identity).unwrap();
        let guard = pinned.acquire_snapshot_guard(false).unwrap();
        let _snapshot = pinned
            .private_snapshot(Instant::now() + Duration::from_secs(2))
            .unwrap();

        let run_writer = |name: &str| {
            let outcome = directory.path().join(name);
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "schema::tests::macos_private_snapshot_does_not_release_the_guard_lock",
                    "--nocapture",
                ])
                .env("SUPER_MEM_INSPECTION_OFD_DATABASE", &database)
                .env("SUPER_MEM_INSPECTION_OFD_OUTCOME", &outcome)
                .status()
                .unwrap();
            assert!(status.success());
            fs::read_to_string(outcome).unwrap()
        };

        assert_eq!(run_writer("while-guarded"), "blocked");
        drop(guard);
        assert_eq!(run_writer("after-drop"), "acquired");
    }

    #[test]
    fn inspection_of_a_closed_wal_store_does_not_materialize_sidecars() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("closed-wal.sqlite3");
        initialized_file(&database);
        let wal = sidecar_path(&database, "-wal");
        let shm = sidecar_path(&database, "-shm");
        assert!(!wal.exists());
        assert!(!shm.exists());
        let before = fs::read(&database).unwrap();

        let inspection = inspect_database(&database, 50).unwrap();

        assert!(inspection.diagnostics.healthy);
        assert_eq!(fs::read(&database).unwrap(), before);
        assert!(!wal.exists());
        assert!(!shm.exists());
    }

    #[test]
    fn windows_memory_snapshot_normalizes_a_closed_wal_header() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("closed-wal-memory.sqlite3");
        initialized_file(&database);
        let source = File::open(&database).unwrap();
        assert!(header_uses_wal(&source).unwrap());
        let length = usize::try_from(source.metadata().unwrap().len()).unwrap();
        let snapshot = read_windows_inspection_snapshot(
            source,
            length,
            Instant::now() + Duration::from_secs(1),
            true,
        )
        .unwrap();
        let mut connection = Connection::open_in_memory().unwrap();

        connection
            .deserialize(rusqlite::MAIN_DB, snapshot, true)
            .unwrap();

        assert_eq!(validate_identity(&connection).unwrap(), SCHEMA_VERSION);
        assert!(connection.execute("DELETE FROM events", []).is_err());
    }

    #[test]
    fn immutable_snapshot_uri_escapes_windows_and_query_delimiters() {
        assert_eq!(
            immutable_sqlite_uri(Path::new(r"C:\Temp\a b?#.sqlite"))
                .unwrap()
                .to_str()
                .unwrap(),
            "file:C:%5CTemp%5Ca%20b%3F%23.sqlite?immutable=1"
        );
    }

    #[test]
    fn inspection_detects_required_schema_damage() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("damaged.sqlite3");
        initialized_file(&database);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE; DROP INDEX feedback_memory;")
            .unwrap();
        drop(connection);

        let inspection = inspect_database(&database, 50).unwrap();
        assert!(!inspection.diagnostics.schema_manifest_ok);
        assert!(!inspection.diagnostics.healthy);
        assert!(
            inspection
                .diagnostics
                .schema_manifest_findings
                .iter()
                .any(|finding| finding == "missing index feedback_memory")
        );
    }

    #[test]
    fn inspection_detects_a_declared_foreign_key_removed_from_schema() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("missing-foreign-key.sqlite3");
        initialized_file(&database);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA wal_checkpoint(TRUNCATE);
                 PRAGMA journal_mode=DELETE;
                 PRAGMA foreign_keys=OFF;
                 ALTER TABLE feedback RENAME TO feedback_old;
                 CREATE TABLE feedback (
                    feedback_id INTEGER PRIMARY KEY AUTOINCREMENT,
                    query_id TEXT,
                    memory_id TEXT NOT NULL,
                    signal TEXT NOT NULL,
                    note TEXT,
                    created_at_ms INTEGER NOT NULL
                 );
                 DROP TABLE feedback_old;
                 CREATE INDEX feedback_memory ON feedback(memory_id, signal);",
            )
            .unwrap();
        drop(connection);

        let inspection = inspect_database(&database, 50).unwrap();
        assert_eq!(inspection.diagnostics.foreign_key_violations, 0);
        assert!(!inspection.diagnostics.schema_manifest_ok);
        assert!(!inspection.diagnostics.healthy);
        assert!(
            inspection
                .diagnostics
                .schema_manifest_findings
                .iter()
                .any(|finding| finding == "changed table feedback")
        );
    }

    #[test]
    fn inspection_detects_a_head_whose_current_revision_is_missing() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("orphaned-head.sqlite3");
        initialized_file(&database);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "INSERT INTO events(
                    event_id,namespace,kind,scope_json,content,attributes_json,
                    trust,occurred_at_ms,ingested_at_ms,content_hash,redaction_count
                 ) VALUES(
                    'event-1','default','memory','{}','body','{}',
                    'observed',1,1,'event-hash',0
                 );
                 INSERT INTO memory_heads(
                    memory_id,namespace,scope_key,kind,state,head_revision,
                    importance,confidence,trust,created_at_ms,updated_at_ms,
                    created_seq,updated_seq
                 ) VALUES(
                    'memory-1','default','scope','fact','active',1,
                    0.5,0.5,'observed',1,1,1,1
                 );
                 INSERT INTO memory_revisions(
                    memory_id,revision,title,body,attributes_json,scope_json,
                    content_hash,recorded_at_ms,recorded_seq
                 ) VALUES(
                    'memory-1',1,'title','body','{}','{}','memory-hash',1,1
                 );
                 INSERT INTO memory_revision_metadata(
                    memory_id,revision,kind,state,importance,confidence,trust,
                    metadata_complete
                 ) VALUES(
                    'memory-1',1,'fact','active',0.5,0.5,'observed',1
                 );
                 PRAGMA foreign_keys=OFF;
                 DELETE FROM memory_revision_metadata WHERE memory_id='memory-1';
                 DELETE FROM memory_revisions WHERE memory_id='memory-1';
                 PRAGMA wal_checkpoint(TRUNCATE);
                 PRAGMA journal_mode=DELETE;",
            )
            .unwrap();
        drop(connection);

        let inspection = inspect_database(&database, 50).unwrap();
        assert!(inspection.diagnostics.quick_check_ok);
        assert_eq!(inspection.diagnostics.foreign_key_violations, 0);
        assert!(inspection.diagnostics.schema_manifest_ok);
        assert!(!inspection.diagnostics.application_invariants_ok);
        assert!(!inspection.diagnostics.healthy);
        assert!(
            inspection
                .diagnostics
                .application_invariant_findings
                .iter()
                .any(|finding| finding == "memory_head_without_head_revision")
        );
    }

    #[test]
    fn inspection_detects_a_head_behind_its_latest_revision() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("stale-head.sqlite3");
        initialized_file(&database);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "INSERT INTO events(
                    event_id,namespace,kind,scope_json,content,attributes_json,
                    trust,occurred_at_ms,ingested_at_ms,content_hash,redaction_count
                 ) VALUES(
                    'event-1','default','memory','{}','body','{}',
                    'observed',1,1,'event-hash',0
                 );
                 INSERT INTO memory_heads(
                    memory_id,namespace,scope_key,kind,state,head_revision,
                    importance,confidence,trust,created_at_ms,updated_at_ms,
                    created_seq,updated_seq
                 ) VALUES(
                    'memory-1','default','scope','fact','active',1,
                    0.5,0.5,'observed',1,2,1,1
                 );
                 INSERT INTO memory_revisions(
                    memory_id,revision,title,body,attributes_json,scope_json,
                    content_hash,recorded_at_ms,recorded_seq
                 ) VALUES
                    ('memory-1',1,'title','old','{}','{}','old-hash',1,1),
                    ('memory-1',2,'title','new','{}','{}','new-hash',2,1);
                 INSERT INTO memory_revision_metadata(
                    memory_id,revision,kind,state,importance,confidence,trust,
                    metadata_complete
                 ) VALUES
                    ('memory-1',1,'fact','active',0.5,0.5,'observed',1),
                    ('memory-1',2,'fact','active',0.5,0.5,'observed',1);
                 PRAGMA wal_checkpoint(TRUNCATE);
                 PRAGMA journal_mode=DELETE;",
            )
            .unwrap();
        drop(connection);

        let inspection = inspect_database(&database, 50).unwrap();
        assert!(inspection.diagnostics.quick_check_ok);
        assert_eq!(inspection.diagnostics.foreign_key_violations, 0);
        assert!(inspection.diagnostics.schema_manifest_ok);
        assert!(!inspection.diagnostics.application_invariants_ok);
        assert!(!inspection.diagnostics.healthy);
        assert!(
            inspection
                .diagnostics
                .application_invariant_findings
                .iter()
                .any(|finding| finding == "memory_head_revision_not_latest")
        );
    }

    #[test]
    fn inspection_progress_deadline_interrupts_pathological_sql_work() {
        let connection = Connection::open_in_memory().unwrap();
        configure_inspection_connection(&connection, Instant::now()).unwrap();
        let started = Instant::now();
        let result = connection.query_row(
            "WITH RECURSIVE values_(value) AS (
                 VALUES(1) UNION ALL SELECT value+1 FROM values_ WHERE value<100000000
             ) SELECT sum(value) FROM values_",
            [],
            |row| row.get::<_, i64>(0),
        );
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn inspection_rejects_oversized_schema_cells_without_changing_the_source() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("oversized-schema.sqlite3");
        initialized_file(&database);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA wal_checkpoint(TRUNCATE);
                 PRAGMA journal_mode=DELETE;
                 PRAGMA writable_schema=ON;
                 UPDATE sqlite_schema
                 SET type=printf('%.*c',5242880,'x')
                 WHERE name='feedback';
                 PRAGMA writable_schema=OFF;
                 PRAGMA schema_version=7;",
            )
            .unwrap();
        drop(connection);
        let before = fs::read(&database).unwrap();
        let started = Instant::now();

        assert!(inspect_database(&database, 50).is_err());

        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(fs::read(&database).unwrap(), before);
    }

    #[test]
    fn inspection_rejects_unexpected_tables_indexes_and_triggers() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("extra-schema.sqlite3");
        initialized_file(&database);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA wal_checkpoint(TRUNCATE);
                 PRAGMA journal_mode=DELETE;
                 CREATE TABLE doctor_rogue(value INTEGER);
                 CREATE INDEX doctor_rogue_index ON doctor_rogue(value);
                 CREATE TRIGGER doctor_rogue_trigger AFTER INSERT ON events
                 BEGIN INSERT INTO doctor_rogue VALUES(NEW.seq); END;",
            )
            .unwrap();
        drop(connection);

        let inspection = inspect_database(&database, 50).unwrap();
        assert!(!inspection.diagnostics.schema_manifest_ok);
        assert!(!inspection.diagnostics.healthy);
        for finding in [
            "unexpected table doctor_rogue",
            "unexpected index doctor_rogue_index",
            "unexpected trigger doctor_rogue_trigger",
        ] {
            assert!(
                inspection
                    .diagnostics
                    .schema_manifest_findings
                    .iter()
                    .any(|actual| actual == finding)
            );
        }

        let reserved_database = directory.path().join("reserved-prefix.sqlite3");
        initialized_file(&reserved_database);
        let connection = Connection::open(&reserved_database).unwrap();
        connection
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
            .unwrap();
        let schema_cookie = connection
            .query_row("PRAGMA schema_version", [], |row| row.get::<_, i64>(0))
            .unwrap();
        connection
            .execute_batch(&format!(
                "PRAGMA writable_schema=ON;
                 INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)
                 VALUES('trigger','sqlite_doctor_rogue','events',0,
                   'CREATE TRIGGER sqlite_doctor_rogue AFTER INSERT ON events BEGIN SELECT 1; END');
                 PRAGMA schema_version={};
                 PRAGMA writable_schema=OFF;",
                schema_cookie + 1
            ))
            .unwrap();
        drop(connection);

        let inspection = inspect_database(&reserved_database, 50).unwrap();
        assert!(!inspection.diagnostics.schema_manifest_ok);
        assert!(
            inspection
                .diagnostics
                .schema_manifest_findings
                .iter()
                .any(|finding| finding == "unexpected trigger sqlite_doctor_rogue")
        );
    }

    #[test]
    fn inspection_rejects_malformed_and_symbolic_link_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let malformed = directory.path().join("malformed.sqlite3");
        fs::write(&malformed, b"not sqlite").unwrap();
        assert!(inspect_database(&malformed, 50).is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let database = directory.path().join("real.sqlite3");
            initialized_file(&database);
            let alias = directory.path().join("alias.sqlite3");
            symlink(&database, &alias).unwrap();
            assert!(inspect_database(&alias, 50).is_err());
        }
    }

    #[test]
    fn inspection_rejects_a_store_exchanged_after_the_callers_preflight() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("memory.sqlite3");
        let replacement = directory.path().join("replacement.sqlite3");
        let original = directory.path().join("original.sqlite3");
        initialized_file(&database);
        initialized_file(&replacement);
        let expected =
            database_file_identity(&database, &fs::symlink_metadata(&database).unwrap()).unwrap();
        let expected_digest = expected.diagnostic_digest();

        fs::rename(&database, &original).unwrap();
        fs::rename(&replacement, &database).unwrap();
        let replacement_identity =
            database_file_identity(&database, &fs::symlink_metadata(&database).unwrap()).unwrap();
        assert_ne!(expected_digest, replacement_identity.diagnostic_digest());

        let error = inspect_database_at_identity(&database, 50, &expected)
            .unwrap_err()
            .to_string();
        assert!(error.contains("changed after the caller's preflight snapshot"));
    }

    #[test]
    fn inspection_refuses_live_wal_without_touching_shared_memory() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("live.sqlite3");
        initialized_file(&database);
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA wal_autocheckpoint=0;
                 CREATE TABLE inspection_live(value INTEGER);
                 INSERT INTO inspection_live VALUES(1);",
            )
            .unwrap();
        let wal = sidecar_path(&database, "-wal");
        let shm = sidecar_path(&database, "-shm");
        let wal_before = fs::read(&wal).unwrap();
        let shm_before = fs::read(&shm).unwrap();

        let error = inspect_database(&database, 50).unwrap_err().to_string();
        assert!(error.contains("live WAL state"));
        assert_eq!(fs::read(&wal).unwrap(), wal_before);
        assert_eq!(fs::read(&shm).unwrap(), shm_before);
        drop(connection);
    }

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
        // Seed v5 derived search state before v6 exists so this exercises the
        // lifecycle and FTS backfill, not merely the post-migration triggers.
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
                INSERT INTO artifacts(
                    artifact_id,namespace,repo_id,path,symbol,content_hash,
                    git_oid,language
                ) VALUES(
                    41,'default','repo','src/migration.rs','migrate',
                    'artifact-hash','','rust'
                ),(
                    42,'default','repo','src/unverifiable.rs','',
                    '','','rust'
                );
                ",
            )
            .unwrap();
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
        let (stored_identity, stored_content) = connection
            .query_row(
                "SELECT identity,content FROM artifact_fingerprints WHERE artifact_id=41",
                [],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .unwrap();
        let (expected_identity, expected_content) = crate::applicability::artifact_fingerprint(
            "repo",
            "src/migration.rs",
            Some("migrate"),
            "artifact-hash",
        )
        .digests();
        assert_eq!(stored_identity, expected_identity);
        assert_eq!(stored_content, expected_content);
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM artifact_fingerprints WHERE artifact_id=42 AND identity IS NULL AND content IS NULL",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1,
            "migration must retain a coverage marker for unverifiable artifacts"
        );
        connection
            .execute("DELETE FROM artifacts WHERE artifact_id=41", [])
            .unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM artifact_fingerprints", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1,
            "derived artifact fingerprints must follow canonical row deletion"
        );
        connection
            .execute("DELETE FROM artifacts WHERE artifact_id=42", [])
            .unwrap();
        assert_eq!(
            connection
                .query_row("SELECT count(*) FROM artifact_fingerprints", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
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
                "UPDATE search_projections SET expansion='' WHERE profile_id='expansion-v1'",
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
        connection
            .execute(
                "UPDATE search_projections SET expansion='another likely query restored' WHERE profile_id='expansion-v1'",
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
