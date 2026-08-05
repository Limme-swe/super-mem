//! Codex and Claude Code command-hook normalization.

use std::{collections::BTreeMap, path::Path};

use anyhow::Context;
use serde_json::{Map, Value, json};
use super_mem_core::{
    CheckpointOutcome, CheckpointRequest, EventKind, ObserveRequest, RecallRequest, TrustLevel,
};

use crate::{
    app::{
        automatic_idempotency_key, capture_scope_artifacts, context_envelope, open_engine_and_scope,
    },
    cli::{HarnessArg, ScopeArgs},
};

const MAX_AUTOMATIC_CAPTURE_BYTES: usize = 64 * 1024;
const CAPTURE_TRUNCATION_MARKER: &str =
    "\n… [super-mem automatic capture truncated; middle omitted] …\n";

/// Processes one hook payload. Any parsing, storage, or recall failure is
/// reported to stderr and converted to the host's neutral `{}` response.
pub fn process(
    database: &Path,
    harness: HarnessArg,
    namespace: &str,
    workspace: Option<&str>,
    input: &str,
) -> Value {
    match process_inner(database, harness, namespace, workspace, input) {
        Ok(response) => response,
        Err(error) => {
            eprintln!(
                "supermem hook {} failed open: {error:#}",
                harness_name(harness)
            );
            json!({})
        }
    }
}

#[allow(clippy::too_many_lines)]
fn process_inner(
    database: &Path,
    harness: HarnessArg,
    namespace: &str,
    workspace: Option<&str>,
    input: &str,
) -> anyhow::Result<Value> {
    let payload: Value = serde_json::from_str(input).context("invalid hook JSON")?;
    let object = payload
        .as_object()
        .context("hook input must be a JSON object")?;
    let event = string(object, "hook_event_name").unwrap_or("Unknown");
    if !matches!(
        event,
        "SessionStart"
            | "UserPromptSubmit"
            | "SubagentStart"
            | "Stop"
            | "SubagentStop"
            | "PostCompact"
            | "PostToolUse"
            | "PostToolUseFailure"
    ) {
        return Ok(json!({}));
    }
    let session = string(object, "session_id").map(str::to_owned);
    let cwd = string(object, "cwd").map(Into::into);
    let scope_arguments = ScopeArgs {
        namespace: namespace.to_owned(),
        workspace: workspace.map(str::to_owned),
        session,
        cwd,
        harness: Some(harness_name(harness).into()),
        ..ScopeArgs::default()
    };
    let (engine, scope) = open_engine_and_scope(database, &scope_arguments)?;

    match event {
        "UserPromptSubmit" => {
            if let Some(prompt) = string(object, "prompt").filter(|value| !value.trim().is_empty())
            {
                let prompt = cap_automatic_capture(prompt);
                let mut attributes = hook_attributes(object, harness, event);
                attributes.insert("role".into(), json!("user"));
                let idempotency_key = hook_idempotency_key(object, harness, event, &prompt);
                engine.observe(ObserveRequest {
                    idempotency_key: Some(idempotency_key),
                    kind: EventKind::ConversationTurn,
                    scope: scope.clone(),
                    content: prompt,
                    attributes,
                    trust: TrustLevel::UserConfirmed,
                    ..ObserveRequest::default()
                })?;
            }
        }
        "Stop" | "SubagentStop" => {
            if let Some(message) =
                string(object, "last_assistant_message").filter(|value| !value.trim().is_empty())
            {
                let message = cap_automatic_capture(message);
                let idempotency_key = hook_idempotency_key(object, harness, event, &message);
                let artifacts = capture_scope_artifacts(&scope, &[], true).unwrap_or_default();
                engine.checkpoint_session(CheckpointRequest {
                    idempotency_key: Some(idempotency_key),
                    scope: scope.clone(),
                    goal: if event == "SubagentStop" {
                        "Complete the delegated coding task"
                    } else {
                        "Complete the current coding turn"
                    }
                    .into(),
                    summary: message,
                    outcome: CheckpointOutcome::Partial,
                    trust: TrustLevel::Agent,
                    tags: vec!["automatic".into(), harness_name(harness).into()],
                    artifacts,
                    ..CheckpointRequest::default()
                })?;
            }
        }
        "PostToolUse" | "PostToolUseFailure" => {
            if let Some(tool_name) = string(object, "tool_name")
                && !is_super_mem_tool(tool_name)
            {
                let command = tool_command(object);
                let content = tool_result_content(object, tool_name, command.as_deref());
                let exit_code = tool_exit_code(object);
                let succeeded = event != "PostToolUseFailure"
                    && exit_code.is_none_or(|code| code == 0)
                    && !tool_response_is_error(object);
                let mut attributes = hook_attributes(object, harness, event);
                attributes.insert("tool_name".into(), json!(tool_name));
                attributes.insert("succeeded".into(), json!(succeeded));
                if let Some(tool_use_id) = string(object, "tool_use_id") {
                    attributes.insert("tool_use_id".into(), json!(tool_use_id));
                }
                if let Some(command) = command.as_deref() {
                    attributes.insert("command".into(), json!(cap_automatic_capture(command)));
                    attributes.insert(
                        "verification".into(),
                        json!(is_verification_command(command)),
                    );
                }
                if let Some(exit_code) = exit_code {
                    attributes.insert("exit_code".into(), json!(exit_code));
                }
                if !succeeded {
                    attributes.insert(
                        "error_fingerprint".into(),
                        json!(automatic_idempotency_key(
                            "hook.tool-error.v1",
                            &[tool_name, command.as_deref().unwrap_or(""), &content],
                        )),
                    );
                }
                engine.observe(ObserveRequest {
                    idempotency_key: Some(hook_idempotency_key(object, harness, event, &content)),
                    kind: tool_event_kind(tool_name),
                    scope: scope.clone(),
                    content,
                    attributes,
                    trust: TrustLevel::ToolVerified,
                    ..ObserveRequest::default()
                })?;
            }
        }
        "PostCompact" => {
            if let Some(summary) =
                string(object, "compact_summary").filter(|value| !value.trim().is_empty())
            {
                let summary = cap_automatic_capture(summary);
                let artifacts = capture_scope_artifacts(&scope, &[], true).unwrap_or_default();
                engine.checkpoint_session(CheckpointRequest {
                    idempotency_key: Some(hook_idempotency_key(object, harness, event, &summary)),
                    scope: scope.clone(),
                    goal: "Continue the coding session after compaction".into(),
                    summary,
                    outcome: CheckpointOutcome::Partial,
                    trust: TrustLevel::Agent,
                    tags: vec!["compaction".into(), harness_name(harness).into()],
                    artifacts,
                    ..CheckpointRequest::default()
                })?;
            }
        }
        _ => {}
    }

    if !matches!(event, "SessionStart" | "UserPromptSubmit" | "SubagentStart") {
        return Ok(json!({}));
    }

    let query = string(object, "prompt")
        .filter(|value| !value.trim().is_empty())
        .map_or_else(
            || "project decisions constraints conventions failures and unfinished work".into(),
            cap_automatic_capture,
        );
    let token_budget = if event == "SubagentStart" {
        1_200
    } else {
        2_000
    };
    let pack = engine.recall(RecallRequest {
        query,
        scope,
        token_budget: Some(token_budget),
        ..RecallRequest::default()
    })?;
    let context = context_envelope(&pack.rendered);
    if context.is_empty() {
        Ok(json!({}))
    } else {
        Ok(response_with_context(event, &context))
    }
}

fn response_with_context(event: &str, context: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": event,
            "additionalContext": context
        }
    })
}

fn is_super_mem_tool(tool_name: &str) -> bool {
    let normalized = tool_name.to_ascii_lowercase();
    normalized.contains("super_mem")
        || normalized.contains("super-mem")
        || normalized.contains("supermem")
        || matches!(
            normalized.as_str(),
            "memory_context" | "memory_record" | "memory_feedback" | "memory_manage"
        )
}

fn tool_event_kind(tool_name: &str) -> EventKind {
    let normalized = tool_name.to_ascii_lowercase();
    let leaf = normalized
        .rsplit(['_', '.', ':', '/'])
        .find(|component| !component.is_empty())
        .unwrap_or(&normalized);
    if matches!(leaf, "bash" | "shell" | "exec" | "command" | "terminal") {
        EventKind::CommandResult
    } else if ["write", "edit", "apply_patch", "notebookedit", "multiedit"]
        .iter()
        .any(|name| normalized.contains(name))
    {
        EventKind::FileChange
    } else {
        EventKind::ToolResult
    }
}

fn tool_command(object: &Map<String, Value>) -> Option<String> {
    object
        .get("tool_input")
        .and_then(Value::as_object)
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)
        .or_else(|| string(object, "command"))
        .filter(|command| !command.trim().is_empty())
        .map(str::to_owned)
}

fn tool_result_content(
    object: &Map<String, Value>,
    tool_name: &str,
    command: Option<&str>,
) -> String {
    let mut parts = vec![format!("Tool: {tool_name}")];
    if let Some(command) = command {
        parts.push(format!("Command: {command}"));
    } else if let Some(input) = object.get("tool_input") {
        parts.push(format!("Input: {}", compact_json(input)));
    }
    if let Some(response) = object
        .get("tool_response")
        .or_else(|| object.get("tool_result"))
        .or_else(|| object.get("error"))
    {
        parts.push(format!("Result: {}", compact_json(response)));
    }
    cap_automatic_capture(&parts.join("\n"))
}

fn compact_json(value: &Value) -> String {
    value.as_str().map_or_else(
        || serde_json::to_string(value).unwrap_or_else(|_| "<unavailable>".into()),
        str::to_owned,
    )
}

fn tool_exit_code(object: &Map<String, Value>) -> Option<i64> {
    object
        .get("exit_code")
        .and_then(Value::as_i64)
        .or_else(|| nested_i64(object, "tool_response", "exit_code"))
        .or_else(|| nested_metadata_i64(object, "tool_response", "exit_code"))
        .or_else(|| nested_i64(object, "tool_result", "exit_code"))
        .or_else(|| nested_metadata_i64(object, "tool_result", "exit_code"))
}

fn nested_i64(object: &Map<String, Value>, parent: &str, field: &str) -> Option<i64> {
    object
        .get(parent)
        .and_then(Value::as_object)
        .and_then(|value| value.get(field))
        .and_then(Value::as_i64)
}

fn nested_metadata_i64(object: &Map<String, Value>, parent: &str, field: &str) -> Option<i64> {
    object
        .get(parent)
        .and_then(Value::as_object)
        .and_then(|value| value.get("metadata"))
        .and_then(Value::as_object)
        .and_then(|value| value.get(field))
        .and_then(Value::as_i64)
}

fn tool_response_is_error(object: &Map<String, Value>) -> bool {
    object.get("is_error").and_then(Value::as_bool) == Some(true)
        || object
            .get("tool_response")
            .and_then(|response| response.get("is_error"))
            .and_then(Value::as_bool)
            == Some(true)
}

fn is_verification_command(command: &str) -> bool {
    command
        .to_ascii_lowercase()
        .split([';', '&', '|'])
        .map(str::trim)
        .any(|segment| {
            [
                "cargo test",
                "cargo nextest",
                "cargo check",
                "cargo clippy",
                "cargo build",
                "pytest",
                "python -m pytest",
                "python3 -m pytest",
                "npm test",
                "npm run test",
                "npm run lint",
                "npm run typecheck",
                "npm run build",
                "pnpm test",
                "pnpm lint",
                "pnpm typecheck",
                "yarn test",
                "go test",
                "go vet",
                "go build",
                "dotnet test",
                "mvn test",
                "gradle test",
                "./gradlew test",
                "tsc",
                "eslint",
                "biome",
                "ruff",
                "mypy",
            ]
            .iter()
            .any(|prefix| segment.starts_with(prefix))
        })
}

fn hook_attributes(
    object: &Map<String, Value>,
    harness: HarnessArg,
    event: &str,
) -> BTreeMap<String, Value> {
    let mut attributes = BTreeMap::from([
        ("harness".into(), json!(harness_name(harness))),
        ("hook_event".into(), json!(event)),
    ]);
    for key in [
        "turn_id",
        "prompt_id",
        "agent_id",
        "agent_type",
        "source",
        "reason",
        "transcript_path",
        "agent_transcript_path",
    ] {
        if let Some(value) = object.get(key).filter(|value| !value.is_null()) {
            attributes.insert(key.into(), value.clone());
        }
    }
    attributes
}

fn hook_idempotency_key(
    object: &Map<String, Value>,
    harness: HarnessArg,
    event: &str,
    content: &str,
) -> String {
    let (session_state, session) =
        string(object, "session_id").map_or(("missing", ""), |session| ("present", session));
    let (turn_source, turn) = ["tool_use_id", "turn_id", "prompt_id", "agent_id"]
        .into_iter()
        .find_map(|key| string(object, key).map(|value| (key, value)))
        .unwrap_or(("missing", ""));
    automatic_idempotency_key(
        "cli.hook.host-event",
        &[
            harness_name(harness),
            session_state,
            session,
            turn_source,
            turn,
            event,
            content,
        ],
    )
}

fn string<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(Value::as_str)
}

pub(crate) fn cap_automatic_capture(value: &str) -> String {
    if value.len() <= MAX_AUTOMATIC_CAPTURE_BYTES {
        return value.to_owned();
    }
    let content_budget = MAX_AUTOMATIC_CAPTURE_BYTES - CAPTURE_TRUNCATION_MARKER.len();
    let mut head_end = content_budget / 2;
    while !value.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let tail_budget = content_budget - head_end;
    let mut tail_start = value.len() - tail_budget;
    while !value.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    format!(
        "{}{}{}",
        &value[..head_end],
        CAPTURE_TRUNCATION_MARKER,
        &value[tail_start..]
    )
}

const fn harness_name(harness: HarnessArg) -> &'static str {
    match harness {
        HarnessArg::Codex => "codex",
        HarnessArg::Claude => "claude-code",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_json_fails_open() {
        let response = process(
            Path::new("/definitely/not/opened"),
            HarnessArg::Codex,
            "default",
            None,
            "{",
        );
        assert_eq!(response, json!({}));
    }

    #[test]
    fn context_response_uses_host_event_name() {
        assert_eq!(
            response_with_context("UserPromptSubmit", "remember this"),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "UserPromptSubmit",
                    "additionalContext": "remember this"
                }
            })
        );
    }

    #[test]
    fn idempotency_is_stable_and_content_sensitive() {
        let object = serde_json::from_value::<Map<String, Value>>(json!({
            "session_id": "s1",
            "turn_id": "t1"
        }))
        .unwrap();
        let first = hook_idempotency_key(&object, HarnessArg::Claude, "Stop", "one");
        let retry = hook_idempotency_key(&object, HarnessArg::Claude, "Stop", "one");
        let changed = hook_idempotency_key(&object, HarnessArg::Claude, "Stop", "two");
        assert_eq!(first, retry);
        assert_ne!(first, changed);
        assert_eq!(first.len(), 68);
    }

    #[test]
    fn idempotency_frames_host_fields_and_bounds_long_identifiers() {
        let left = serde_json::from_value::<Map<String, Value>>(json!({
            "session_id": "s:t",
            "turn_id": "u"
        }))
        .unwrap();
        let right = serde_json::from_value::<Map<String, Value>>(json!({
            "session_id": "s",
            "turn_id": "t:u"
        }))
        .unwrap();
        let literal_unknown = serde_json::from_value::<Map<String, Value>>(json!({
            "session_id": "unknown",
            "turn_id": "unknown"
        }))
        .unwrap();
        let missing = Map::new();
        let long = "host:id:".repeat(64);
        let long_fields = serde_json::from_value::<Map<String, Value>>(json!({
            "session_id": long,
            "turn_id": long
        }))
        .unwrap();

        assert_ne!(
            hook_idempotency_key(&left, HarnessArg::Claude, "Stop", "same"),
            hook_idempotency_key(&right, HarnessArg::Claude, "Stop", "same")
        );
        assert_ne!(
            hook_idempotency_key(&literal_unknown, HarnessArg::Claude, "Stop", "same"),
            hook_idempotency_key(&missing, HarnessArg::Claude, "Stop", "same")
        );
        assert!(
            hook_idempotency_key(&long_fields, HarnessArg::Claude, "Stop", "same").len() <= 256
        );
    }

    #[test]
    fn automatic_capture_cap_is_utf8_safe_and_preserves_head_and_tail() {
        let value = format!("HEAD{}TAIL", "λ".repeat(MAX_AUTOMATIC_CAPTURE_BYTES));
        let capped = cap_automatic_capture(&value);
        assert!(capped.len() <= MAX_AUTOMATIC_CAPTURE_BYTES);
        assert!(capped.starts_with("HEAD"));
        assert!(capped.ends_with("TAIL"));
        assert!(capped.contains(CAPTURE_TRUNCATION_MARKER));
        assert_eq!(cap_automatic_capture("short"), "short");
    }

    #[test]
    fn tool_events_ground_session_checkpoints_inside_the_configured_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let database = temp.path().join("memory.sqlite3");
        let cwd = temp.path().to_string_lossy().into_owned();
        let tool_payload = json!({
            "hook_event_name": "PostToolUse",
            "session_id": "session-1",
            "tool_use_id": "tool-1",
            "cwd": cwd.clone(),
            "tool_name": "Bash",
            "tool_input": { "command": "cargo test migration" },
            "tool_response": {
                "stdout": "all migration tests passed",
                "exit_code": 0
            }
        });
        assert_eq!(
            process(
                &database,
                HarnessArg::Claude,
                "team-a",
                Some("workspace-a"),
                &tool_payload.to_string(),
            ),
            json!({})
        );
        let stop_payload = json!({
            "hook_event_name": "Stop",
            "session_id": "session-1",
            "turn_id": "turn-1",
            "cwd": cwd,
            "last_assistant_message": "Serialized schema migration startup."
        });
        process(
            &database,
            HarnessArg::Claude,
            "team-a",
            Some("workspace-a"),
            &stop_payload.to_string(),
        );

        let engine = crate::app::open_engine(&database).unwrap();
        let pack = engine
            .recall(RecallRequest {
                query: "cargo test migration".into(),
                scope: super_mem_core::Scope {
                    namespace: "team-a".into(),
                    workspace_id: Some("workspace-a".into()),
                    session_id: Some("session-1".into()),
                    ..super_mem_core::Scope::default()
                },
                ..RecallRequest::default()
            })
            .unwrap();
        assert!(pack.rendered.contains("all migration tests passed"));
        assert!(pack.hits.iter().any(|hit| hit.memory.evidence.len() >= 2));

        let isolated = engine
            .recall(RecallRequest {
                query: "cargo test migration".into(),
                scope: super_mem_core::Scope {
                    namespace: "team-a".into(),
                    workspace_id: Some("workspace-b".into()),
                    ..super_mem_core::Scope::default()
                },
                ..RecallRequest::default()
            })
            .unwrap();
        assert!(isolated.hits.is_empty());
    }

    #[test]
    fn memory_tool_results_are_not_captured_recursively() {
        assert!(is_super_mem_tool("mcp__super_mem__memory_context"));
        assert!(is_super_mem_tool("mcp__supermem__memory_context"));
        assert!(is_super_mem_tool("memory_record"));
        assert!(!is_super_mem_tool("Bash"));
        assert_eq!(
            tool_event_kind("functions.exec_command"),
            EventKind::CommandResult
        );
        assert!(is_verification_command("cd crates/core && cargo test"));
    }
}
