# Codex adapter

## Install

1. Install supermem on PATH.
2. From a repository checkout, open `/plugins` in Codex CLI, choose the Super Mem repository marketplace, and install Super Mem. In ChatGPT Work or Codex desktop, restart the app and install it from the Super Mem source in Plugins.
3. Review the hook commands before trusting them.
4. Verify hooks with /hooks and MCP with codex mcp list or /mcp verbose.

For a manual project install, copy hooks/hooks.json to .codex/hooks.json and merge config.toml into the active Codex configuration.

The included MCP command uses --root . and otherwise reads the standard `SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, and `SUPER_MEM_WORKSPACE` environment variables. Use an absolute trusted root if Codex may start elsewhere.

Hooks and MCP are separate processes. Set `SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, and `SUPER_MEM_WORKSPACE` in the environment that launches Codex when selecting a custom store or scope. The bundled MCP configuration forwards those variables so both processes resolve the same values; the defaults remain namespace `default` with no workspace.

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
