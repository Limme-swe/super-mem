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
