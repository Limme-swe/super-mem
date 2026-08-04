//! End-to-end tests for the CLI, hooks, and MCP stdio surface.

use std::{fs, path::Path};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::TempDir;

fn command(database: &Path) -> Command {
    let mut command = Command::cargo_bin("supermem").expect("binary");
    command.arg("--db").arg(database);
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
    }
    assert!(pi.contains("--summary-stdin"));
    assert!(pi.contains("getLeafId()"));
    assert!(pi.contains(":compact:${String(entry.id)}"));
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

#[cfg(unix)]
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

#[cfg(unix)]
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
fn purge_is_conservatively_refused_on_windows() {
    let temp = TempDir::new().unwrap();
    let database = temp.path().join("memory.sqlite3");
    command(&database)
        .args(["remember", "--body", "Preserve this memory."])
        .assert()
        .success();

    command(&database)
        .args(["purge", "--yes"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "safe purge is not supported on this platform",
        ));

    assert!(database.is_file());
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
