# Codex adapter

## Install

1. Install supermem on PATH.
2. Install this directory as a Codex plugin, or copy hooks/hooks.json to .codex/hooks.json and merge config.toml into the active Codex configuration.
3. Review the hook commands before trusting them.
4. Verify hooks with /hooks and MCP with codex mcp list or /mcp verbose.

The included MCP command uses --root . --namespace default. Use an absolute trusted root if Codex may start elsewhere, and add --workspace when separate workspaces share a repository.

## Captured events

| Event | Action |
| --- | --- |
| SessionStart | Recall context. |
| UserPromptSubmit | Record the prompt and recall context. |
| SubagentStart | Recall a smaller context packet. |
| Stop / SubagentStop | Create one partial checkpoint from the finalized assistant message. |

The checkpoint stores the final message once; no duplicate assistant observation is written. transcript_path is provenance only because Codex does not guarantee its format.

Automatic text is capped at 64 KiB. Hook failures are fail-open.
