# OpenCode adapter

The manifest is configured for @super-mem/opencode, but the package is not published. For a checkout:

1. Install supermem on PATH.
2. Copy src/index.ts to .opencode/plugins/super-mem.ts.
3. Merge opencode.json into the project configuration and remove the unpublished package name from the plugin array.

## Behavior

- chat.message records the prompt and recalls context in one CLI call.
- Recalled evidence is appended to the per-message system prompt without creating a synthetic user message.
- session.idle checkpoints the latest finalized assistant message.
- The optional compaction hook adds recall during OpenCode's experimental compaction event.
- Individual tool outcomes are not captured yet.

The MCP command pins --root . --namespace default. Use an absolute trusted root when the launch directory may differ, and add --workspace when needed.

Automatic text is capped at 64 KiB before it crosses stdin. Subprocess failures and timeouts are fail-open.
