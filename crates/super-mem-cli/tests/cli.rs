//! End-to-end tests for the CLI, hooks, and MCP stdio surface.

use std::{
    fs,
    path::Path,
    process::Command as ProcessCommand,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn command(database: &Path) -> Command {
    let mut command = Command::cargo_bin("supermem").expect("binary");
    let binary_directory = Path::new(command.get_program())
        .parent()
        .expect("test binary directory")
        .to_path_buf();
    let mut path = vec![binary_directory];
    if let Some(existing) = std::env::var_os("PATH") {
        path.extend(std::env::split_paths(&existing));
    }
    command.env("PATH", std::env::join_paths(path).expect("test PATH"));
    command.arg("--db").arg(database);
    command
}

fn git(repository: &Path, arguments: &[&str]) {
    assert!(
        ProcessCommand::new("git")
            .args(arguments)
            .current_dir(repository)
            .status()
            .unwrap()
            .success()
    );
}

fn init_git_repository(repository: &Path) {
    fs::create_dir_all(repository).unwrap();
    git(repository, &["init", "--quiet"]);
    git(
        repository,
        &["config", "user.email", "test@example.invalid"],
    );
    git(repository, &["config", "user.name", "Test"]);
    git(
        repository,
        &["commit", "--allow-empty", "--quiet", "-m", "initial"],
    );
}

#[cfg(unix)]
fn command_with_fake_git(database: &Path, directory: &Path, script: &str) -> Command {
    use std::os::unix::fs::{PermissionsExt, symlink};

    fs::create_dir_all(directory).unwrap();
    let git = directory.join("git");
    fs::write(&git, script).unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o700)).unwrap();

    let mut command = Command::cargo_bin("supermem").expect("binary");
    let binary = Path::new(command.get_program()).to_path_buf();
    let path_binary = directory.join("supermem");
    if !path_binary.exists() && fs::hard_link(&binary, &path_binary).is_err() {
        symlink(&binary, &path_binary).unwrap();
    }
    command.env("PATH", directory).arg("--db").arg(database);
    command
}

#[test]
fn help_exposes_the_complete_surface() {
    Command::cargo_bin("supermem")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(
            predicate::str::contains("remember")
                .and(predicate::str::contains("checkpoint"))
                .and(predicate::str::contains("feedback"))
                .and(predicate::str::contains("doctor"))
                .and(predicate::str::contains("export"))
                .and(predicate::str::contains("import"))
                .and(predicate::str::contains("hook"))
                .and(predicate::str::contains("mcp")),
        );

    Command::cargo_bin("supermem")
        .unwrap()
        .args(["mcp", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("--root")
                .and(predicate::str::contains("--namespace"))
                .and(predicate::str::contains("--workspace")),
        );

    Command::cargo_bin("supermem")
        .unwrap()
        .args(["index", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("artifact-status"));
}

#[test]
fn artifact_projection_status_is_available_without_a_search_profile() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database)
        .args(["--json", "index", "artifact-status"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"referenced\": 0")
                .and(predicate::str::contains("\"canonical\": 0"))
                .and(predicate::str::contains("\"projected\": 0"))
                .and(predicate::str::contains("\"valid\": 0"))
                .and(predicate::str::contains("\"orphaned\": 0"))
                .and(predicate::str::contains("\"degraded\": false")),
        );
}

#[test]
fn doctor_reports_database_and_repository_health_explicitly() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    command(&database).arg("init").assert().success();

    command(&database)
        .args(["--json", "doctor", "--cwd", repository.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"ok\": true")
                .and(predicate::str::contains("\"healthy\": true"))
                .and(predicate::str::contains("\"state\": \"repository\""))
                .and(predicate::str::contains("\"writer_lock_available\": true")),
        );
}

#[test]
fn doctor_accepts_a_relative_database_path_without_creating_an_alias() {
    let temp = TempDir::new().unwrap();
    init_git_repository(temp.path());
    let relative_database = Path::new("memory.sqlite3");
    command(relative_database)
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    command(relative_database)
        .current_dir(temp.path())
        .args(["--json", "doctor", "--cwd", "."])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("\"ok\": true")
                .and(predicate::str::contains("\"healthy\": true")),
        );
    assert!(temp.path().join(relative_database).is_file());
}

#[test]
fn doctor_fails_closed_when_git_metadata_is_broken() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let repository = temp.path().join("broken-repository");
    command(&database).arg("init").assert().success();
    fs::create_dir_all(&repository).unwrap();
    fs::write(repository.join(".git"), "gitdir: missing\n").unwrap();

    command(&database)
        .args(["--json", "doctor", "--cwd", repository.to_str().unwrap()])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("\"ok\": false")
                .and(predicate::str::contains("\"state\": \"repository_error\""))
                .and(predicate::str::contains("repository_discovery")),
        )
        .stderr(predicate::str::contains("repository_discovery"));
}

#[test]
fn doctor_reports_when_git_is_not_on_path() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let empty_path = temp.path().join("empty-path");
    fs::create_dir(&empty_path).unwrap();
    command(&database).arg("init").assert().success();

    command(&database)
        .env("PATH", &empty_path)
        .args(["--json", "doctor", "--cwd", temp.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("\"state\": \"git_unavailable\"")
                .and(predicate::str::contains("repository_discovery")),
        );
}

#[test]
fn doctor_does_not_create_a_missing_database() {
    let temp = TempDir::new().unwrap();
    let parent = temp.path().join("missing-parent");
    let database = parent.join("memory.sqlite3");

    command(&database)
        .args(["--json", "doctor", "--cwd", temp.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("database_missing")
                .and(predicate::str::contains("\"opened_read_only\": false")),
        );
    assert!(!parent.exists());
}

#[test]
fn doctor_reports_malformed_database_as_json() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    fs::write(&database, b"not a sqlite database").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    }

    command(&database)
        .args(["--json", "doctor", "--cwd", temp.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("database_inspection")
                .and(predicate::str::contains("\"inspection_error\":")),
        );
}

#[test]
fn doctor_reports_writer_contention_without_normal_open() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database).arg("init").assert().success();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
        .unwrap();
    connection.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let started = Instant::now();

    command(&database)
        .args(["--json", "doctor", "--cwd", temp.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("database_integrity_or_writer_lock")
                .and(predicate::str::contains("\"writer_lock_available\": false")),
        );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "doctor exceeded its bounded writer probe"
    );
    connection.execute_batch("ROLLBACK;").unwrap();
}

#[test]
fn doctor_surfaces_canonical_foreign_key_damage() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database).arg("init").assert().success();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             INSERT INTO feedback(memory_id,signal,created_at_ms)
             VALUES('missing-memory','used',0);",
        )
        .unwrap();
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
        .unwrap();
    drop(connection);

    command(&database)
        .args(["--json", "doctor", "--cwd", temp.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("\"foreign_key_violations\": 1").and(
                predicate::str::contains("database_integrity_or_writer_lock"),
            ),
        );
}

#[test]
fn doctor_surfaces_a_head_whose_current_revision_is_missing() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database)
        .args(["remember", "--body", "The current revision must exist."])
        .assert()
        .success();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             DELETE FROM memory_revision_metadata;
             DELETE FROM memory_revisions;
             PRAGMA wal_checkpoint(TRUNCATE);
             PRAGMA journal_mode=DELETE;",
        )
        .unwrap();
    drop(connection);

    command(&database)
        .args(["--json", "doctor", "--cwd", temp.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("\"application_invariants_ok\": false")
                .and(predicate::str::contains(
                    "memory_head_without_head_revision",
                ))
                .and(predicate::str::contains(
                    "database_integrity_or_writer_lock",
                )),
        );
}

#[test]
fn doctor_surfaces_a_head_behind_its_latest_revision() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    for body in ["The first revision.", "The corrected revision."] {
        command(&database)
            .args([
                "remember",
                "--body",
                body,
                "--canonical-key",
                "doctor-stale-head",
                "--cwd",
            ])
            .arg(temp.path())
            .assert()
            .success();
    }
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "UPDATE memory_heads SET head_revision=1;
             PRAGMA wal_checkpoint(TRUNCATE);
             PRAGMA journal_mode=DELETE;",
        )
        .unwrap();
    drop(connection);

    command(&database)
        .args(["--json", "doctor", "--cwd", temp.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("\"application_invariants_ok\": false")
                .and(predicate::str::contains("memory_head_revision_not_latest"))
                .and(predicate::str::contains(
                    "database_integrity_or_writer_lock",
                )),
        );
}

#[cfg(any(unix, windows))]
#[test]
fn doctor_rejects_a_hard_linked_database() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database).arg("init").assert().success();
    fs::hard_link(&database, temp.path().join("database-alias.sqlite3")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&database, fs::Permissions::from_mode(0o640)).unwrap();
    }
    let before = fs::read(&database).unwrap();

    command(&database)
        .args(["--json", "doctor", "--cwd", temp.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("database_file_security")
                .and(predicate::str::contains("\"hard_links\": 2")),
        );
    assert_eq!(fs::read(&database).unwrap(), before);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&database).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}

#[cfg(unix)]
#[test]
fn doctor_does_not_open_a_database_through_a_parent_symlink() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let temp = TempDir::new().unwrap();
    let target_directory = temp.path().join("target");
    fs::create_dir(&target_directory).unwrap();
    let target_database = target_directory.join("memory.sqlite3");
    command(&target_database).arg("init").assert().success();
    fs::set_permissions(&target_database, fs::Permissions::from_mode(0o640)).unwrap();
    let before = fs::read(&target_database).unwrap();
    let alias = temp.path().join("database-alias");
    symlink(&target_directory, &alias).unwrap();
    let alias_database = alias.join("memory.sqlite3");

    command(&alias_database)
        .args(["--json", "doctor", "--cwd", temp.path().to_str().unwrap()])
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("database_file_security")
                .and(predicate::str::contains("\"path_components_safe\": false")),
        );
    assert_eq!(fs::read(&target_database).unwrap(), before);
    assert_eq!(
        fs::metadata(&target_database).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[cfg(unix)]
#[test]
fn doctor_bounds_hung_git_and_terminates_its_process_group() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database).arg("init").assert().success();
    let fake_bin = temp.path().join("fake-bin");
    let started = Instant::now();

    command_with_fake_git(&database, &fake_bin, "#!/bin/sh\n/bin/sleep 30 &\nwait\n")
        .args(["--json", "doctor", "--cwd"])
        .arg(temp.path())
        .assert()
        .failure()
        .stdout(predicate::str::contains("\"state\": \"git_timeout\""));
    assert!(started.elapsed() < Duration::from_secs(3));

    let started = Instant::now();
    command_with_fake_git(&database, &fake_bin, "#!/bin/sh\n/bin/sleep 30 &\nexit 0\n")
        .args(["--json", "doctor", "--cwd"])
        .arg(temp.path())
        .assert()
        .failure();
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "a descendant retaining Git pipes blocked doctor"
    );
}

#[cfg(unix)]
#[test]
fn doctor_rejects_oversized_git_output_without_echoing_it() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database).arg("init").assert().success();
    let repository = temp.path().join("repository");
    fs::create_dir_all(repository.join(".git")).unwrap();
    let oversized = temp.path().join("oversized-remote");
    fs::write(&oversized, vec![b'a'; 16 * 1_024 * 1_024]).unwrap();
    let script = format!(
        "#!/bin/sh\ncase \"$1 $2\" in\n  '--version ') echo 'git version test' ;;\n  'rev-parse --show-toplevel') echo '{}' ;;\n  'rev-parse --git-common-dir') echo '.git' ;;\n  'status --porcelain=v2') printf '# branch.oid 0123456789012345678901234567890123456789\\0# branch.head main\\0' ;;\n  'config --get') exec /bin/cat '{}' ;;\n  *) exit 2 ;;\nesac\n",
        repository.display(),
        oversized.display()
    );
    let output = command_with_fake_git(&database, &temp.path().join("fake-bin"), &script)
        .args(["--json", "doctor", "--cwd"])
        .arg(&repository)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.len() < 128 * 1_024);
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["repository"]["state"], "git_output_limit");
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn doctor_serializes_non_utf8_paths_as_valid_json() {
    use std::os::unix::ffi::OsStringExt;

    let temp = TempDir::new().unwrap();
    let original = temp.path().join("memory.sqlite3");
    command(&original).arg("init").assert().success();
    let database = temp.path().join(std::ffi::OsString::from_vec(
        b"memory-\xff.sqlite3".to_vec(),
    ));
    fs::rename(&original, &database).unwrap();
    let cwd = temp
        .path()
        .join(std::ffi::OsString::from_vec(b"cwd-\xfe".to_vec()));
    fs::create_dir(&cwd).unwrap();

    let output = command(&database)
        .args(["--json", "doctor", "--cwd"])
        .arg(&cwd)
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["database"]["path"]["native_encoding"], "unix_bytes");
    assert_eq!(report["repository"]["cwd"]["native_encoding"], "unix_bytes");
}

#[test]
fn doctor_redacts_scope_environment_values() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    git(
        &repository,
        &["checkout", "-b", "ghp_branch_DO_NOT_ECHO_123456789"],
    );
    command(&database).arg("init").assert().success();

    let output = command(&database)
        .env("SUPER_MEM_NAMESPACE", "ghp_namespace_secret")
        .env("SUPER_MEM_WORKSPACE", "sk-workspace-secret")
        .env("SUPER_MEM_DB", "/tmp/password=database-secret")
        .args(["--json", "doctor", "--cwd"])
        .arg(&repository)
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("ghp_namespace_secret"));
    assert!(!stdout.contains("sk-workspace-secret"));
    assert!(!stdout.contains("database-secret"));
    assert!(!stdout.contains("DO_NOT_ECHO"));
    assert!(stdout.contains("\"values_redacted\": true"));
}

#[test]
fn doctor_never_emits_git_remote_credentials_or_paths() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    command(&database).arg("init").assert().success();

    for remote in [
        "https://example.test/org/ghp_DO_NOT_ECHO_123456789/repo.git?auth=secret",
        "git@example.test:org/scp_secret_DO_NOT_ECHO/repo.git",
        "/tmp/local_secret_DO_NOT_ECHO/repo.git",
    ] {
        git(&repository, &["config", "remote.origin.url", remote]);
        let output = command(&database)
            .args(["--json", "doctor", "--cwd"])
            .arg(&repository)
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(!stdout.contains("DO_NOT_ECHO"));
        assert!(!stdout.contains("auth=secret"));
        assert!(stdout.contains("\"remote_present\": true"));
    }
}

#[cfg(unix)]
#[test]
fn doctor_preserves_invalid_and_live_sqlite_sidecars() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let invalid_database = temp.path().join("invalid-wal.sqlite3");
    command(&invalid_database).arg("init").assert().success();
    let invalid_wal = Path::new(&format!("{}-wal", invalid_database.display())).to_path_buf();
    fs::write(&invalid_wal, vec![0_u8; 4_096]).unwrap();
    fs::set_permissions(&invalid_wal, fs::Permissions::from_mode(0o600)).unwrap();
    let invalid_before = fs::read(&invalid_wal).unwrap();
    command(&invalid_database)
        .args(["--json", "doctor", "--cwd"])
        .arg(temp.path())
        .assert()
        .failure()
        .stdout(predicate::str::contains("database_file_security"));
    assert_eq!(fs::read(&invalid_wal).unwrap(), invalid_before);

    let live_database = temp.path().join("live-wal.sqlite3");
    command(&live_database).arg("init").assert().success();
    let connection = rusqlite::Connection::open(&live_database).unwrap();
    connection
        .execute_batch("PRAGMA wal_autocheckpoint=0; CREATE TABLE doctor_live(value); INSERT INTO doctor_live VALUES(1);")
        .unwrap();
    let live_wal = Path::new(&format!("{}-wal", live_database.display())).to_path_buf();
    let live_shm = Path::new(&format!("{}-shm", live_database.display())).to_path_buf();
    let wal_before = fs::read(&live_wal).unwrap();
    let shm_before = fs::read(&live_shm).unwrap();
    command(&live_database)
        .args(["--json", "doctor", "--cwd"])
        .arg(temp.path())
        .assert()
        .failure()
        .stdout(predicate::str::contains("database_live_wal"));
    assert_eq!(fs::read(&live_wal).unwrap(), wal_before);
    assert_eq!(fs::read(&live_shm).unwrap(), shm_before);
    drop(connection);

    let journal_database = temp.path().join("hot-journal.sqlite3");
    command(&journal_database).arg("init").assert().success();
    let connection = rusqlite::Connection::open(&journal_database).unwrap();
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
        .unwrap();
    drop(connection);
    let journal = Path::new(&format!("{}-journal", journal_database.display())).to_path_buf();
    fs::write(&journal, b"preserve crash evidence").unwrap();
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o600)).unwrap();
    let database_before = fs::read(&journal_database).unwrap();
    let journal_before = fs::read(&journal).unwrap();
    command(&journal_database)
        .args(["--json", "doctor", "--cwd"])
        .arg(temp.path())
        .assert()
        .failure()
        .stdout(predicate::str::contains("database_recovery_required"));
    assert_eq!(fs::read(&journal_database).unwrap(), database_before);
    assert_eq!(fs::read(&journal).unwrap(), journal_before);
}

#[test]
fn doctor_does_not_change_delete_journal_mode_or_database_bytes() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    command(&database).arg("init").assert().success();
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA journal_mode=DELETE;")
        .unwrap();
    drop(connection);
    let before = fs::read(&database).unwrap();

    command(&database)
        .args(["--json", "doctor", "--cwd"])
        .arg(&repository)
        .assert()
        .success();
    assert_eq!(fs::read(&database).unwrap(), before);
    let connection = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "delete"
    );
}

#[cfg(unix)]
#[test]
#[allow(unsafe_code)]
fn doctor_fails_closed_for_an_unwritable_store_or_parent() {
    use std::os::unix::fs::PermissionsExt;

    if unsafe { libc::geteuid() } == 0 {
        // Root legitimately bypasses mode-bit write denial. CI exercises this
        // regression as its ordinary unprivileged runner account.
        return;
    }

    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    let store = temp.path().join("store");
    let database = store.join("memory.sqlite3");
    init_git_repository(&repository);
    fs::create_dir(&store).unwrap();
    command(&database).arg("init").assert().success();
    let before = fs::read(&database).unwrap();

    fs::set_permissions(&database, fs::Permissions::from_mode(0o400)).unwrap();
    command(&database)
        .args(["--json", "doctor", "--cwd"])
        .arg(&repository)
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("database_integrity_or_writer_lock")
                .and(predicate::str::contains("database is not writable")),
        );
    assert_eq!(fs::read(&database).unwrap(), before);

    fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&store, fs::Permissions::from_mode(0o500)).unwrap();
    command(&database)
        .args(["--json", "doctor", "--cwd"])
        .arg(&repository)
        .assert()
        .failure()
        .stdout(
            predicate::str::contains("database_integrity_or_writer_lock").and(
                predicate::str::contains("parent does not grant effective write"),
            ),
        );
    assert_eq!(fs::read(&database).unwrap(), before);
    fs::set_permissions(&store, fs::Permissions::from_mode(0o700)).unwrap();
}

#[cfg(unix)]
#[test]
fn doctor_preserves_a_repository_root_ending_in_newline() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let repository = temp.path().join("repository\n");
    init_git_repository(&repository);
    command(&database).arg("init").assert().success();

    let output = command(&database)
        .args(["--json", "doctor", "--cwd"])
        .arg(&repository)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        report["repository"]["probe"]["root"],
        repository.canonicalize().unwrap().to_str().unwrap()
    );
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn init_serializes_a_non_utf8_database_path_without_panicking() {
    use std::os::unix::ffi::OsStringExt;

    let temp = TempDir::new().unwrap();
    let database = temp
        .path()
        .join(std::ffi::OsString::from_vec(b"init-\xff.sqlite3".to_vec()));
    let output = command(&database)
        .args(["--json", "init"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["database"]["native_encoding"], "unix_bytes");
}

#[test]
fn search_profile_cli_lifecycle_is_explicit_and_reversible() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database)
        .args([
            "--json",
            "index",
            "add-profile",
            "--profile-id",
            "cli-expansion-v1",
            "--model-digest",
            "fixture-generator-v1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"active\": true"));

    command(&database)
        .args(["index", "list-profiles"])
        .assert()
        .success()
        .stdout(predicate::str::contains("cli-expansion-v1"));
    command(&database)
        .args([
            "--json",
            "index",
            "deactivate",
            "--profile-id",
            "cli-expansion-v1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"active\": false"));
    command(&database)
        .args([
            "--json",
            "index",
            "activate",
            "--profile-id",
            "cli-expansion-v1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"active\": true"));

    command(&database)
        .args([
            "index",
            "remove-profile",
            "--profile-id",
            "cli-expansion-v1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("without --yes"));
    command(&database)
        .args([
            "--json",
            "index",
            "remove-profile",
            "--profile-id",
            "cli-expansion-v1",
            "--yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"removed\": \"cli-expansion-v1\"",
        ));
    command(&database)
        .args(["index", "list-profiles"])
        .assert()
        .success()
        .stdout(predicate::eq("[]\n"));
}

#[test]
fn async_library_entrypoint_remains_usable() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let arguments = vec![
        std::ffi::OsString::from("supermem"),
        std::ffi::OsString::from("--db"),
        database.into_os_string(),
        std::ffi::OsString::from("status"),
        std::ffi::OsString::from("--quiet"),
    ];
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(super_mem::run_from(arguments))
        .unwrap();
}

#[test]
fn remember_recall_inspect_and_retract_round_trip() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let output = command(&database)
        .args([
            "--json",
            "remember",
            "--body-stdin",
            "--title",
            "Package manager",
            "--kind",
            "decision",
            "--canonical-key",
            "package-manager",
            "--cwd",
        ])
        .arg(temp.path())
        .write_stdin("Use pnpm for dependency management in this project.")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let receipt: Value = serde_json::from_slice(&output).unwrap();
    let id = receipt["memory_ids"][0].as_str().unwrap();

    command(&database)
        .args(["recall", "--query-stdin", "--format", "context", "--cwd"])
        .arg(temp.path())
        .write_stdin("Which package manager should I use?")
        .assert()
        .success()
        .stdout(predicate::str::contains("pnpm"))
        .stdout(predicate::str::contains("<super-mem-context>"));

    command(&database)
        .args(["inspect", id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Package manager"));

    command(&database)
        .args(["inspect", id, "--history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"metadata_complete\": true"))
        .stdout(predicate::str::contains("\"events\""));

    command(&database)
        .args([
            "retract",
            id,
            "--reason",
            "project migrated package managers",
        ])
        .assert()
        .success();

    command(&database)
        .args(["--json", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"retracted_memories\": 1"));
}

#[test]
fn observe_retries_are_idempotent() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    for _ in 0..2 {
        command(&database)
            .args([
                "--json",
                "observe",
                "--kind",
                "assistant_final",
                "--content-stdin",
                "--event-id",
                "host-message-7",
                "--harness",
                "test",
                "--cwd",
            ])
            .arg(temp.path())
            .write_stdin("Tests pass.")
            .assert()
            .success();
    }
    command(&database)
        .args(["--json", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"events\": 1"));
}

#[test]
fn observe_accepts_structured_tool_result_metadata() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database)
        .args([
            "observe",
            "--kind",
            "command_result",
            "--content-stdin",
            "--event-id",
            "tool-call-1",
            "--tool-name",
            "Bash",
            "--succeeded",
            "false",
            "--verification",
            "true",
            "--error-fingerprint",
            "smerr1:test",
            "--session",
            "session-1",
            "--cwd",
        ])
        .arg(temp.path())
        .write_stdin("cargo test failed")
        .assert()
        .success();
}

#[test]
fn generated_observe_keys_are_bounded_content_sensitive_and_unambiguous() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    for (harness, session, event_id, content) in [
        ("host:a", "session", "event", "same content"),
        ("host", "a:session", "a:event", "same content"),
    ] {
        command(&database)
            .args([
                "observe",
                "--kind",
                "assistant_final",
                "--content-stdin",
                "--event-id",
                event_id,
                "--harness",
                harness,
                "--session",
                session,
                "--cwd",
            ])
            .arg(temp.path())
            .write_stdin(content)
            .assert()
            .success();
    }

    let long = "host:id:".repeat(50);
    for content in ["long retry", "long retry", "changed content"] {
        command(&database)
            .args([
                "observe",
                "--kind",
                "assistant_final",
                "--content-stdin",
                "--event-id",
                &long,
                "--harness",
                &long,
                "--session",
                &long,
                "--cwd",
            ])
            .arg(temp.path())
            .write_stdin(content)
            .assert()
            .success();
    }

    let output = command(&database)
        .args(["--json", "status"])
        .output()
        .unwrap();
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["events"], 4);
}

#[test]
fn checkpoint_accepts_summary_from_stdin() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database)
        .args([
            "checkpoint",
            "--goal",
            "Fix flaky test",
            "--summary-stdin",
            "--outcome",
            "success",
            "--verification",
            "cargo test",
        ])
        .write_stdin("Replaced the timing assertion with an event barrier.")
        .assert()
        .success();
    command(&database)
        .args([
            "recall",
            "--query",
            "event barrier flaky timing",
            "--format",
            "context",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("event barrier"));
}

#[test]
fn process_adapters_pipe_agent_content_instead_of_putting_it_in_argv() {
    let opencode = include_str!("../../../adapters/opencode/src/index.ts");
    let pi = include_str!("../../../adapters/pi/extensions/super-mem.ts");
    for adapter in [opencode, pi] {
        assert!(adapter.contains("--query-stdin"));
        assert!(adapter.contains("--observe-prompt"));
        assert!(adapter.contains("MAX_CAPTURE_BYTES = 64 * 1024"));
        assert!(adapter.contains("automatic capture truncated"));
        assert!(!adapter.contains("assistant_final"));
        assert!(!adapter.contains("\"--content\","));
        assert!(!adapter.contains("\"--query\","));
        assert!(adapter.contains("automaticIdempotencyKey"));
        assert!(adapter.contains("super-mem automatic idempotency key derivation v1"));
        assert!(adapter.contains("return `sm1:${hash.digest(\"hex\")}`"));
    }
    assert!(pi.contains("--summary-stdin"));
    assert!(pi.contains("getLeafId()"));
    assert!(pi.contains("pi.session-compaction"));
    assert!(pi.contains("pi.agent-checkpoint"));
    assert!(opencode.contains("opencode.assistant-checkpoint"));
    assert!(!pi.contains("`pi:${ctx.sessionManager.getSessionId()}"));
    assert!(!opencode.contains("`opencode:${sessionID}:${eventID}`"));
}

#[test]
fn recall_observes_prompt_in_the_same_idempotent_invocation() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database)
        .args([
            "remember",
            "--body",
            "Use cargo nextest for the test suite.",
            "--harness",
            "adapter-test",
            "--session",
            "session-7",
            "--cwd",
        ])
        .arg(temp.path())
        .assert()
        .success();

    for _ in 0..2 {
        command(&database)
            .args([
                "recall",
                "--query-stdin",
                "--observe-prompt",
                "--event-id",
                "message-42",
                "--harness",
                "adapter-test",
                "--session",
                "session-7",
                "--cwd",
            ])
            .arg(temp.path())
            .write_stdin("cargo nextest")
            .assert()
            .success()
            .stdout(predicate::str::contains("cargo nextest"));
    }
    command(&database)
        .args([
            "recall",
            "--query-stdin",
            "--observe-prompt",
            "--event-id",
            "message-42",
            "--harness",
            "adapter-test",
            "--session",
            "session-7",
            "--cwd",
        ])
        .arg(temp.path())
        .write_stdin("cargo nextest test suite")
        .assert()
        .success()
        .stdout(predicate::str::contains("cargo nextest"));

    let output = command(&database)
        .args(["--json", "status"])
        .output()
        .unwrap();
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["events"], 3);
}

#[test]
fn observed_prompt_keys_frame_boundaries_and_bound_long_host_ids() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    for (harness, session, event_id) in [
        ("host:a", "session", "event"),
        ("host", "a:session", "event"),
    ] {
        command(&database)
            .args([
                "recall",
                "--query-stdin",
                "--observe-prompt",
                "--event-id",
                event_id,
                "--harness",
                harness,
                "--session",
                session,
                "--cwd",
            ])
            .arg(temp.path())
            .write_stdin("boundary test")
            .assert()
            .success();
    }

    let long = "host:id:".repeat(50);
    for _ in 0..2 {
        command(&database)
            .args([
                "recall",
                "--query-stdin",
                "--observe-prompt",
                "--event-id",
                &long,
                "--harness",
                &long,
                "--session",
                &long,
                "--cwd",
            ])
            .arg(temp.path())
            .write_stdin("long identifier retry")
            .assert()
            .success();
    }

    let output = command(&database)
        .args(["--json", "status"])
        .output()
        .unwrap();
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["events"], 3);
}

#[test]
fn export_and_import_preserve_memory() {
    let temp = TempDir::new().unwrap();
    let first = temp.path().join("first.sqlite3");
    let second = temp.path().join("second.sqlite3");
    let export = temp.path().join("memory.jsonl");

    command(&first)
        .args([
            "remember",
            "--body",
            "The release command is cargo xtask release.",
            "--kind",
            "procedure",
        ])
        .assert()
        .success();
    command(&first)
        .arg("export")
        .arg("--output")
        .arg(&export)
        .assert()
        .success();
    assert!(!fs::read_to_string(&export).unwrap().is_empty());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&export).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    command(&first)
        .arg("export")
        .assert()
        .success()
        .stdout(predicate::str::contains("super_mem_export"));

    command(&second)
        .arg("import")
        .arg(&export)
        .assert()
        .success();
    command(&second)
        .args([
            "recall",
            "--query",
            "How do we release?",
            "--format",
            "context",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("cargo xtask release"));
}

#[test]
fn malformed_hook_input_is_protocol_clean_and_fail_open() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database)
        .args(["hook", "codex"])
        .write_stdin("not-json")
        .assert()
        .success()
        .stdout("{}\n");
}

#[test]
fn hook_fails_open_when_no_database_location_can_be_resolved() {
    Command::cargo_bin("supermem")
        .unwrap()
        .env_remove("SUPER_MEM_DB")
        .env_remove("XDG_DATA_HOME")
        .env_remove("LOCALAPPDATA")
        .env_remove("HOME")
        .args(["hook", "codex"])
        .write_stdin("{}")
        .assert()
        .success()
        .stdout("{}\n");
}

#[test]
fn default_database_uses_the_platform_data_directory() {
    let temp = TempDir::new().unwrap();
    let home = temp.path().join("home");
    let local_app_data = temp.path().join("local-app-data");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&local_app_data).unwrap();

    Command::cargo_bin("supermem")
        .unwrap()
        .env_remove("SUPER_MEM_DB")
        .env("XDG_DATA_HOME", "relative-xdg-must-be-ignored")
        .env("HOME", &home)
        .env("LOCALAPPDATA", &local_app_data)
        .arg("init")
        .assert()
        .success();

    #[cfg(windows)]
    let expected = local_app_data.join("super-mem/memory.sqlite3");
    #[cfg(target_os = "macos")]
    let expected = home.join("Library/Application Support/super-mem/memory.sqlite3");
    #[cfg(not(any(windows, target_os = "macos")))]
    let expected = home.join(".local/share/super-mem/memory.sqlite3");
    assert!(expected.is_file(), "{} was not created", expected.display());
}

#[test]
fn changed_artifact_is_stale_on_the_native_platform() {
    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository with spaces and ünicode");
    init_git_repository(&repository);
    let artifact = repository.join("tracked.txt");
    fs::write(&artifact, "original\n").unwrap();
    git(&repository, &["add", "tracked.txt"]);
    git(&repository, &["commit", "--quiet", "-m", "add artifact"]);
    let database = temp.path().join("memory.sqlite3");

    command(&database)
        .args([
            "remember",
            "--body",
            "The native freshness sentinel is copper-lark.",
            "--file",
            "tracked.txt",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .success();
    fs::write(&artifact, "changed\n").unwrap();

    command(&database)
        .args([
            "recall",
            "--query",
            "native freshness sentinel",
            "--format",
            "context",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .success()
        .stdout(predicate::str::contains("copper-lark").not());
    command(&database)
        .args([
            "recall",
            "--query",
            "native freshness sentinel",
            "--format",
            "context",
            "--include-stale",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .success()
        .stdout(predicate::str::contains("copper-lark"))
        .stdout(predicate::str::contains("; stale]"));
}

#[test]
fn descendant_commit_remains_compatible_on_the_native_platform() {
    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository with spaces and ünicode");
    init_git_repository(&repository);
    let database = temp.path().join("memory.sqlite3");

    command(&database)
        .args([
            "remember",
            "--body",
            "The ancestry sentinel is silver-heron.",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .success();
    fs::write(repository.join("later.txt"), "later commit\n").unwrap();
    git(&repository, &["add", "later.txt"]);
    git(&repository, &["commit", "--quiet", "-m", "descendant"]);

    command(&database)
        .args([
            "recall",
            "--query",
            "ancestry sentinel",
            "--format",
            "context",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .success()
        .stdout(predicate::str::contains("silver-heron"))
        .stdout(predicate::str::contains("; compatible]"));
}

#[cfg(any(unix, windows))]
#[test]
fn repo_local_nonignored_database_is_rejected_before_creation() {
    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    let database = repository.join("memory.sqlite3");

    command(&database)
        .args(["remember", "--body", "Must not self-stale.", "--cwd"])
        .arg(&repository)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not ignored by Git"));
    assert!(!database.exists());

    command(&database)
        .args(["recall", "--query", "self stale", "--cwd"])
        .arg(&repository)
        .assert()
        .failure()
        .stderr(predicate::str::contains("not ignored by Git"));
    assert!(!database.exists());

    let start = serde_json::json!({
        "hook_event_name": "SessionStart",
        "session_id": "session-1",
        "cwd": &repository
    });
    command(&database)
        .args(["hook", "codex"])
        .write_stdin(start.to_string())
        .assert()
        .success()
        .stdout("{}\n")
        .stderr(predicate::str::contains("not ignored by Git"));
    assert!(!database.exists());

    command(&database)
        .args(["mcp", "--root"])
        .arg(&repository)
        .args(["--namespace", "mcp-test"])
        .write_stdin("")
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("not ignored by Git"));
    assert!(!database.exists());
}

#[cfg(any(unix, windows))]
#[test]
fn fully_ignored_repo_local_database_recalls_as_exact() {
    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    fs::write(
        repository.join(".gitignore"),
        "/memory.sqlite3\n/memory.sqlite3-wal\n/memory.sqlite3-shm\n/memory.sqlite3-journal\n",
    )
    .unwrap();
    git(&repository, &["add", ".gitignore"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "ignore memory database"],
    );
    let database = repository.join("memory.sqlite3");

    command(&database)
        .args([
            "remember",
            "--body",
            "The exact in-repository sentinel is violet-finch.",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .success();
    command(&database)
        .args([
            "recall",
            "--query",
            "in-repository sentinel",
            "--format",
            "context",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .success()
        .stdout(predicate::str::contains("violet-finch"))
        .stdout(predicate::str::contains("; exact]"));
}

#[cfg(any(unix, windows))]
#[test]
fn repo_local_database_requires_every_sqlite_sidecar_to_be_ignored() {
    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    fs::write(repository.join(".gitignore"), "/memory.sqlite3\n").unwrap();
    git(&repository, &["add", ".gitignore"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "incomplete database ignore"],
    );
    let database = repository.join("memory.sqlite3");

    command(&database)
        .args(["remember", "--body", "Must not self-stale.", "--cwd"])
        .arg(&repository)
        .assert()
        .failure()
        .stderr(
            predicate::str::contains("memory.sqlite3-wal")
                .and(predicate::str::contains("not ignored by Git")),
        );
    assert!(!database.exists());
}

#[cfg(any(unix, windows))]
#[test]
fn tracked_repo_local_database_is_always_rejected() {
    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    let database = repository.join("memory.sqlite3");
    command(&database).arg("init").assert().success();
    git(&repository, &["add", "memory.sqlite3"]);
    git(&repository, &["commit", "--quiet", "-m", "track database"]);

    command(&database)
        .args([
            "remember",
            "--body",
            "Must not mutate tracked data.",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .failure()
        .stderr(predicate::str::contains("tracked by Git"));
}

#[cfg(any(unix, windows))]
#[test]
fn ignored_hardlinked_database_cannot_mutate_a_tracked_alias() {
    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    fs::write(repository.join(".gitignore"), "/memory.sqlite3*\n").unwrap();
    git(&repository, &["add", ".gitignore"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "ignore memory database"],
    );
    let database = repository.join("memory.sqlite3");
    let tracked_alias = repository.join("tracked.sqlite3");
    command(&database).arg("init").assert().success();
    fs::hard_link(&database, &tracked_alias).unwrap();
    git(&repository, &["add", "tracked.sqlite3"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "track database alias"],
    );
    let before = fs::read(&tracked_alias).unwrap();

    command(&database)
        .args([
            "remember",
            "--body",
            "Must not mutate the tracked alias.",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .failure()
        .stderr(predicate::str::contains("multiple hard links"));

    assert_eq!(fs::read(&tracked_alias).unwrap(), before);
    let status = ProcessCommand::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(status.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn ignored_database_symlink_cannot_redirect_into_a_tracked_alias() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    fs::write(repository.join(".gitignore"), "/memory.sqlite3*\n").unwrap();
    git(&repository, &["add", ".gitignore"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "ignore memory database"],
    );
    let external = temp.path().join("external.sqlite3");
    let database = repository.join("memory.sqlite3");
    let tracked_alias = repository.join("tracked.sqlite3");
    command(&external).arg("init").assert().success();
    fs::hard_link(&external, &tracked_alias).unwrap();
    symlink(&external, &database).unwrap();
    git(&repository, &["add", "tracked.sqlite3"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "track redirected database alias"],
    );
    let before = fs::read(&tracked_alias).unwrap();

    command(&database)
        .args([
            "remember",
            "--body",
            "Must not follow the database symlink.",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .failure()
        .stderr(predicate::str::contains("symbolic-link component"));

    assert!(
        fs::symlink_metadata(&database)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&tracked_alias).unwrap(), before);
    let status = ProcessCommand::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(status.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn ignored_parent_symlink_cannot_hide_a_redirected_database() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    fs::write(repository.join(".gitignore"), "/.memory-store\n").unwrap();
    git(&repository, &["add", ".gitignore"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "ignore memory store"],
    );
    let external_store = temp.path().join("external-store");
    fs::create_dir(&external_store).unwrap();
    let external = external_store.join("memory.sqlite3");
    let store_link = repository.join(".memory-store");
    let database = store_link.join("memory.sqlite3");
    let tracked_alias = repository.join("tracked.sqlite3");
    command(&external).arg("init").assert().success();
    fs::hard_link(&external, &tracked_alias).unwrap();
    symlink(&external_store, &store_link).unwrap();
    git(&repository, &["add", "tracked.sqlite3"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "track parent-redirect alias"],
    );
    let before = fs::read(&tracked_alias).unwrap();

    command(&database)
        .args([
            "remember",
            "--body",
            "Must not follow the parent symlink.",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .failure()
        .stderr(predicate::str::contains("symbolic-link component"));

    assert!(
        fs::symlink_metadata(&store_link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(&tracked_alias).unwrap(), before);
    let status = ProcessCommand::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(status.stdout.is_empty());
}

#[cfg(windows)]
#[test]
fn ignored_parent_junction_is_rejected_for_scoped_use_and_purge() {
    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    fs::write(repository.join(".gitignore"), "/.memory-store\n").unwrap();
    git(&repository, &["add", ".gitignore"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "ignore memory store junction"],
    );

    let external_store = temp.path().join("external-store");
    fs::create_dir(&external_store).unwrap();
    let external = external_store.join("memory.sqlite3");
    command(&external).arg("init").assert().success();
    let before = fs::read(&external).unwrap();
    let store_link = repository.join(".memory-store");
    let junction = ProcessCommand::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&store_link)
        .arg(&external_store)
        .output()
        .unwrap();
    assert!(
        junction.status.success(),
        "mklink failed: {}",
        String::from_utf8_lossy(&junction.stderr)
    );
    let database = store_link.join("memory.sqlite3");

    command(&database)
        .args([
            "remember",
            "--body",
            "Must not follow a Windows directory junction.",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .failure()
        .stderr(predicate::str::contains("reparse point"));
    command(&database)
        .args(["purge", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("reparse point"));

    assert_eq!(fs::read(&external).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn parent_directory_component_cannot_bypass_symlink_validation() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    fs::write(repository.join(".gitignore"), "/link\n/memory.sqlite3*\n").unwrap();
    git(&repository, &["add", ".gitignore"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "ignore redirected memory paths"],
    );
    let external_store = temp.path().join("external-store");
    let external_subdirectory = external_store.join("subdirectory");
    fs::create_dir_all(&external_subdirectory).unwrap();
    let external = external_store.join("memory.sqlite3");
    let tracked_alias = repository.join("tracked.sqlite3");
    command(&external).arg("init").assert().success();
    fs::hard_link(&external, &tracked_alias).unwrap();
    symlink(&external_subdirectory, repository.join("link")).unwrap();
    git(&repository, &["add", "tracked.sqlite3"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "track parent-component alias"],
    );
    let before = fs::read(&tracked_alias).unwrap();
    let database = repository.join("link/../memory.sqlite3");

    command(&database)
        .args([
            "remember",
            "--body",
            "Must not normalize through a symlink.",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .failure()
        .stderr(predicate::str::contains("must not contain '..' components"));

    assert_eq!(fs::read(&tracked_alias).unwrap(), before);
    let status = ProcessCommand::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(status.stdout.is_empty());
}

#[cfg(not(any(unix, windows)))]
#[test]
fn repo_local_database_is_conservatively_rejected_without_safe_link_counts() {
    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    fs::write(repository.join(".gitignore"), "/memory.sqlite3*\n").unwrap();
    git(&repository, &["add", ".gitignore"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "ignore memory database"],
    );
    let database = repository.join("memory.sqlite3");

    command(&database)
        .args(["remember", "--body", "Must remain external.", "--cwd"])
        .arg(&repository)
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "repo-local databases are not supported on this platform",
        ));
    assert!(!database.exists());
}

#[cfg(any(unix, windows))]
#[test]
fn purge_refuses_to_delete_an_unrelated_file() {
    let temp = TempDir::new().unwrap();
    let unrelated = temp.path().join("unrelated.sqlite3");
    let original = b"this file belongs to another application";
    fs::write(&unrelated, original).unwrap();

    command(&unrelated)
        .args(["purge", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not a Super Mem database"));

    assert_eq!(fs::read(&unrelated).unwrap(), original);
}

#[cfg(unix)]
#[test]
fn purge_refuses_a_symlink_and_preserves_the_store() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let alias = temp.path().join("memory-alias.sqlite3");
    command(&database)
        .args(["remember", "--body", "Preserve this memory."])
        .assert()
        .success();
    symlink(&database, &alias).unwrap();

    command(&alias)
        .args(["purge", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("symbolic link"));

    assert!(
        fs::symlink_metadata(&alias)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(database.is_file());
    command(&database)
        .args(["--json", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"active_memories\": 1"));
}

#[cfg(any(unix, windows))]
#[test]
fn purge_refuses_a_hard_link_and_preserves_every_name() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let alias = temp.path().join("memory-alias.sqlite3");
    command(&database)
        .args(["remember", "--body", "Preserve this memory."])
        .assert()
        .success();
    fs::hard_link(&database, &alias).unwrap();

    command(&alias)
        .args(["purge", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("multiple hard links"));

    assert!(alias.is_file());
    assert!(database.is_file());
    command(&database)
        .args(["--json", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"active_memories\": 1"));
}

#[cfg(windows)]
#[test]
fn purge_removes_a_windows_store_after_alias_verification() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database)
        .args(["remember", "--body", "Preserve this memory."])
        .assert()
        .success();

    command(&database)
        .args(["purge", "--yes"])
        .assert()
        .success();

    assert!(!database.exists());
}

#[cfg(any(unix, windows))]
#[test]
fn external_database_hardlink_cannot_mutate_a_tracked_alias() {
    let temp = TempDir::new().unwrap();
    let repository = temp.path().join("repository");
    init_git_repository(&repository);
    let database = temp.path().join("external.sqlite3");
    let tracked_alias = repository.join("tracked.sqlite3");
    command(&database).arg("init").assert().success();
    fs::hard_link(&database, &tracked_alias).unwrap();
    git(&repository, &["add", "tracked.sqlite3"]);
    git(
        &repository,
        &["commit", "--quiet", "-m", "track external database alias"],
    );
    let before = fs::read(&tracked_alias).unwrap();

    command(&database)
        .args([
            "remember",
            "--body",
            "Must not mutate an external tracked alias.",
            "--cwd",
        ])
        .arg(&repository)
        .assert()
        .failure()
        .stderr(predicate::str::contains("multiple hard links"));

    assert_eq!(fs::read(&tracked_alias).unwrap(), before);
    let status = ProcessCommand::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&repository)
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(status.stdout.is_empty());
}

#[test]
fn automatic_stop_checkpoint_is_recalled_in_a_new_session() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let stop = serde_json::json!({
        "hook_event_name": "Stop",
        "session_id": "old-session",
        "turn_id": "turn-7",
        "cwd": temp.path(),
        "last_assistant_message": "Use pnpm for this repository; npm caused lockfile drift."
    });
    command(&database)
        .args(["hook", "codex"])
        .write_stdin(stop.to_string())
        .assert()
        .success()
        .stdout("{}\n");

    let status = command(&database)
        .args(["--json", "status"])
        .output()
        .unwrap();
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["events"], 1);
    assert_eq!(status["active_memories"], 1);

    let prompt = serde_json::json!({
        "hook_event_name": "UserPromptSubmit",
        "session_id": "new-session",
        "turn_id": "turn-1",
        "cwd": temp.path(),
        "prompt": "Which package manager should I use?"
    });
    let output = command(&database)
        .args(["hook", "codex"])
        .write_stdin(prompt.to_string())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("pnpm"));
    assert_eq!(output.matches("<super-mem-context>").count(), 1);
}

#[test]
fn hooks_and_mcp_share_database_namespace_and_workspace_environment() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("scoped-memory.sqlite3");
    let scope_environment = [
        ("SUPER_MEM_DB", database.as_os_str()),
        ("SUPER_MEM_NAMESPACE", std::ffi::OsStr::new("team-memory")),
        (
            "SUPER_MEM_WORKSPACE",
            std::ffi::OsStr::new("workspace-blue"),
        ),
    ];
    let stop = serde_json::json!({
        "hook_event_name": "Stop",
        "session_id": "host-session",
        "turn_id": "turn-9",
        "cwd": temp.path(),
        "last_assistant_message": "The cross-surface scope sentinel is amber-kingfisher."
    });
    Command::cargo_bin("supermem")
        .unwrap()
        .envs(scope_environment)
        .args(["hook", "codex"])
        .write_stdin(stop.to_string())
        .assert()
        .success()
        .stdout("{}\n");

    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"0\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"memory_context\",\"arguments\":{\"query\":\"cross-surface scope sentinel\"}}}\n",
    );
    let output = Command::cargo_bin("supermem")
        .unwrap()
        .envs(scope_environment)
        .args(["mcp", "--root"])
        .arg(temp.path())
        .write_stdin(input)
        .timeout(Duration::from_secs(7))
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .get_output()
        .stdout
        .clone();
    let response = String::from_utf8(output).unwrap();
    let call: Value = response
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .find(|value: &Value| value["id"] == 2)
        .expect("tools/call response");
    let text = call["result"]["content"][0]["text"]
        .as_str()
        .expect("text tool result");
    assert!(text.contains("amber-kingfisher"));
}

#[test]
fn mcp_startup_survives_short_sqlite_writer_contention() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database)
        .args(["status", "--quiet"])
        .assert()
        .success();

    let lock_database = database.clone();
    let (locked_sender, locked_receiver) = mpsc::channel();
    let holder = thread::spawn(move || {
        let connection = rusqlite::Connection::open(lock_database).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        locked_sender.send(()).unwrap();
        thread::sleep(Duration::from_millis(750));
        connection.execute_batch("ROLLBACK").unwrap();
    });
    locked_receiver.recv().unwrap();

    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"0\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
    );
    let started = Instant::now();
    let output = command(&database)
        .args(["mcp", "--root"])
        .arg(temp.path())
        .write_stdin(input)
        .timeout(Duration::from_secs(7))
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .get_output()
        .stdout
        .clone();
    holder.join().unwrap();
    assert!(started.elapsed() >= Duration::from_millis(600));
    let response = String::from_utf8(output).unwrap();
    assert!(
        response.lines().any(|line| {
            serde_json::from_str::<Value>(line).is_ok_and(|value| value["id"] == 1)
        })
    );
}

#[test]
fn hook_keys_frame_boundaries_and_bound_long_host_ids() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    for (session, turn) in [("session:a", "turn"), ("session", "a:turn")] {
        let stop = serde_json::json!({
            "hook_event_name": "Stop",
            "session_id": session,
            "turn_id": turn,
            "cwd": temp.path(),
            "last_assistant_message": "The boundary-sensitive checkpoint is complete."
        });
        command(&database)
            .args(["hook", "codex"])
            .write_stdin(stop.to_string())
            .assert()
            .success()
            .stdout("{}\n");
    }

    let long = "host:id:".repeat(50);
    let long_stop = serde_json::json!({
        "hook_event_name": "Stop",
        "session_id": &long,
        "turn_id": &long,
        "cwd": temp.path(),
        "last_assistant_message": "The long-identifier checkpoint is complete."
    });
    for _ in 0..2 {
        command(&database)
            .args(["hook", "codex"])
            .write_stdin(long_stop.to_string())
            .assert()
            .success()
            .stdout("{}\n");
    }

    let output = command(&database)
        .args(["--json", "status"])
        .output()
        .unwrap();
    let status: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["events"], 3);
    assert_eq!(status["active_memories"], 3);
}

#[test]
fn mcp_lists_exactly_four_annotated_tools_in_stable_order() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"0\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
    );
    let output = command(&database)
        .args(["mcp", "--root"])
        .arg(temp.path())
        .args(["--namespace", "mcp-test"])
        .write_stdin(input)
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let response = String::from_utf8(output).unwrap();
    let list: Value = response
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|value| value["id"] == 2)
        .expect("tools/list response");
    let tools = list["result"]["tools"].as_array().unwrap();
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "memory_context",
            "memory_feedback",
            "memory_manage",
            "memory_record"
        ]
    );
    assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);
    assert_eq!(tools[2]["annotations"]["destructiveHint"], true);
    assert!(!response.contains("memory_purge"));
    for tool in tools {
        let schema = &tool["inputSchema"];
        assert!(schema_has_property(schema, "session_id"));
        for forbidden in ["namespace", "cwd", "repo_id", "workspace_id"] {
            assert!(!schema_has_property(schema, forbidden), "{forbidden}");
        }
    }
}

#[test]
fn mcp_stdio_call_is_protocol_clean_and_shuts_down_on_eof() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database)
        .args([
            "remember",
            "--body",
            "The MCP protocol sentinel is cobalt-orchid.",
            "--namespace",
            "mcp-test",
            "--cwd",
        ])
        .arg(temp.path())
        .assert()
        .success();

    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"0\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"memory_context\",\"arguments\":{\"query\":\"protocol sentinel\"}}}\n",
    );
    let output = command(&database)
        .args(["mcp", "--root"])
        .arg(temp.path())
        .args(["--namespace", "mcp-test"])
        .write_stdin(input)
        .timeout(std::time::Duration::from_secs(5))
        .assert()
        .success()
        .stderr(predicate::str::is_empty())
        .get_output()
        .stdout
        .clone();
    let response = String::from_utf8(output).unwrap();
    let messages = response
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("stdout JSON-RPC message"))
        .collect::<Vec<_>>();
    let call = messages
        .iter()
        .find(|value| value["id"] == 2)
        .expect("tools/call response");
    assert_ne!(call["result"]["isError"], true);
    let text = call["result"]["content"][0]["text"]
        .as_str()
        .expect("text tool result");
    assert!(text.contains("cobalt-orchid"));
    assert_eq!(text.matches("<super-mem-context>").count(), 1);
}

fn schema_has_property(value: &Value, expected: &str) -> bool {
    match value {
        Value::Object(object) => {
            object
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key(expected))
                || object
                    .values()
                    .any(|value| schema_has_property(value, expected))
        }
        Value::Array(values) => values
            .iter()
            .any(|value| schema_has_property(value, expected)),
        _ => false,
    }
}
