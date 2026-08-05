# OpenCode adapter

The manifest is configured for @super-mem/opencode, but the package is not published. For a checkout:

1. Install supermem on PATH.
2. Copy src/index.ts to .opencode/plugins/super-mem.ts.
3. Merge opencode.json into the project configuration and remove the unpublished package name from the plugin array.

## Behavior

- chat.message records the prompt and recalls context in one CLI call.
- Recalled evidence is appended to the per-message system prompt without creating a synthetic user message.
- tool.execute.after records command results, file changes, and other tool outcomes. Test, lint, type-check, and build commands are marked as verification evidence.
- session.idle checkpoints the latest finalized assistant message.
- The optional compaction hook adds recall during OpenCode's experimental compaction event.

The MCP command pins --root . and otherwise reads the standard store and scope environment variables. Use an absolute trusted root when the launch directory may differ.

Set `SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, and `SUPER_MEM_WORKSPACE` in the environment that launches OpenCode when selecting a custom store or scope. The plugin's CLI calls and the MCP process must use the same values.

Automatic prompt and assistant text is capped at 64 KiB before it crosses stdin. Tool commands and results are sent through stdin rather than process arguments; the Rust core bounds and redacts them before storage. Memory tools are excluded from automatic tool capture. Subprocess failures and timeouts are fail-open.
