# Claude Code adapter

## Install

Install supermem on PATH, then copy .mcp.json and .claude/settings.json into the project or merge them with the matching user configuration.

The MCP server can also be registered with:

~~~sh
claude mcp add --scope project --transport stdio super_mem --   supermem mcp --root . --namespace default
~~~

Verify the connection with claude mcp list and /mcp.

The included command pins --root to `${CLAUDE_PROJECT_DIR:-.}` and uses namespace `default`. Use an absolute trusted root for user-level configuration and add `--workspace` when needed.

Hooks and MCP are separate processes. If you change the MCP `--namespace` or `--workspace`, set `SUPER_MEM_NAMESPACE` and `SUPER_MEM_WORKSPACE` to the same values in the environment that launches Claude Code. Hooks otherwise use namespace `default` with no workspace.

## Captured events

| Event | Action |
| --- | --- |
| SessionStart | Recall context. |
| UserPromptSubmit | Record the prompt and recall context. |
| SubagentStart | Recall context for the subagent. |
| PostToolUse / PostToolUseFailure | Record successful and failed non-memory tool outcomes. |
| Stop / SubagentStop | Checkpoint the finalized assistant message. |
| PostCompact | Store the supplied compact summary. |

The adapter uses last_assistant_message because transcript writes may lag. It stores one checkpoint rather than a duplicate assistant observation.

Automatic text is capped at 64 KiB. Hook failures are fail-open.
