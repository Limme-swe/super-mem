# Claude Code adapter

## Install

Install supermem on PATH, then copy .mcp.json and .claude/settings.json into the project or merge them with the matching user configuration.

The MCP server can also be registered with:

~~~sh
claude mcp add --scope project --transport stdio super_mem -- supermem mcp --root .
~~~

Verify the connection with claude mcp list and /mcp.

The included command pins --root to `${CLAUDE_PROJECT_DIR:-.}`. Database, namespace, and workspace use the standard environment variables, with namespace `default` and no workspace when they are unset. Use an absolute trusted root for user-level configuration.

Hooks and MCP are separate processes. Set `SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, and `SUPER_MEM_WORKSPACE` in the environment that launches Claude Code when selecting a custom store or scope so both processes resolve the same values.

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
