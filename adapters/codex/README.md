# Codex adapter

## Install

1. Install supermem on PATH.
2. From a repository checkout, open `/plugins` in Codex CLI, choose the Super Mem repository marketplace, and install Super Mem. In ChatGPT Work or Codex desktop, restart the app and install it from the Super Mem source in Plugins.
3. Review the hook commands before trusting them.
4. Verify hooks with /hooks and MCP with codex mcp list or /mcp verbose.

For a manual project install, copy hooks/hooks.json to .codex/hooks.json and merge config.toml into the active Codex configuration.

The included MCP command uses --root . --namespace default. Use an absolute trusted root if Codex may start elsewhere, and add --workspace when separate workspaces share a repository.

Hooks and MCP are separate processes. If you change the MCP `--namespace` or `--workspace`, set `SUPER_MEM_NAMESPACE` and `SUPER_MEM_WORKSPACE` to the same values in the environment that launches Codex. Hooks otherwise use namespace `default` with no workspace.

## Captured events

| Event | Action |
| --- | --- |
| SessionStart | Recall context. |
| UserPromptSubmit | Record the prompt and recall context. |
| SubagentStart | Recall a smaller context packet. |
| PostToolUse | Record the outcome of completed non-memory tools. |
| Stop / SubagentStop | Create one partial checkpoint from the finalized assistant message. |

The checkpoint stores the final message once; no duplicate assistant observation is written. transcript_path is provenance only because Codex does not guarantee its format.

Automatic text is capped at 64 KiB. Hook failures are fail-open.
