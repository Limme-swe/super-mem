//! Codex and Claude Code command-hook normalization.

use std::{collections::BTreeMap, path::Path};

use anyhow::Context;
use serde_json::{Map, Value, json};
use super_mem_core::{
    CheckpointOutcome, CheckpointRequest, EventKind, ObserveRequest, RecallRequest, TrustLevel,
};

use crate::{
    app::{automatic_idempotency_key, context_envelope, open_engine_and_scope},
    cli::{HarnessArg, ScopeArgs},
};

const MAX_AUTOMATIC_CAPTURE_BYTES: usize = 64 * 1024;
const CAPTURE_TRUNCATION_MARKER: &str =
    "\n… [super-mem automatic capture truncated; middle omitted] …\n";

/// Processes one hook payload. Any parsing, storage, or recall failure is
/// reported to stderr and converted to the host's neutral `{}` response.
pub fn process(database: &Path, harness: HarnessArg, input: &str) -> Value {
    match process_inner(database, harness, input) {
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
fn process_inner(database: &Path, harness: HarnessArg, input: &str) -> anyhow::Result<Value> {
    let payload: Value = serde_json::from_str(input).context("invalid hook JSON")?;
    let object = payload
        .as_object()
        .context("hook input must be a JSON object")?;
    let event = string(object, "hook_event_name").unwrap_or("Unknown");
    let session = string(object, "session_id").map(str::to_owned);
    let cwd = string(object, "cwd").map(Into::into);
    let scope_arguments = ScopeArgs {
        namespace: "default".into(),
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
                engine.checkpoint(CheckpointRequest {
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
                    ..CheckpointRequest::default()
                })?;
            }
        }
        "PostCompact" => {
            if let Some(summary) =
                string(object, "compact_summary").filter(|value| !value.trim().is_empty())
            {
                let summary = cap_automatic_capture(summary);
                engine.checkpoint(CheckpointRequest {
                    idempotency_key: Some(hook_idempotency_key(object, harness, event, &summary)),
                    scope: scope.clone(),
                    goal: "Continue the coding session after compaction".into(),
                    summary,
                    outcome: CheckpointOutcome::Partial,
                    trust: TrustLevel::Agent,
                    tags: vec!["compaction".into(), harness_name(harness).into()],
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
    let (turn_source, turn) = ["turn_id", "prompt_id", "agent_id"]
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
        let response = process(Path::new("/definitely/not/opened"), HarnessArg::Codex, "{");
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
}
