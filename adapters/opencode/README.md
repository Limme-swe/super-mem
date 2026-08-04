# OpenCode adapter

Install the `supermem` binary on `PATH` first. Then install
`@super-mem/opencode` and merge `opencode.json` into the project config.
For local development, place `src/index.ts` in `.opencode/plugins/super-mem.ts`
and remove the package name from the `plugin` array.

The stable `chat.message` hook adds recalled memory to the per-message system
prompt without creating a synthetic user message. One `recall --observe-prompt`
process captures the prompt and returns context with one Git discovery.
`session.idle` retrieves the
last finalized assistant message through the public SDK and writes one partial
checkpoint without a duplicate observation. Automatic prompt and assistant
text is capped at 64 KiB before it crosses stdin. The compaction hook is
experimental and can be removed independently.

Subprocess errors and timeouts fail open. A future resident `supermem` daemon
will be used internally by the same CLI calls; no plugin API change is planned.

The MCP command pins `--root . --namespace default` in `opencode.json`. Use an
absolute trusted root when the launch directory may differ from the project,
and add `--workspace <id>` for another hard boundary. Model tool input cannot
change the pinned root, repository, namespace, or workspace.
