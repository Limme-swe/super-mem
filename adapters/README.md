# Harness adapters

The reference adapters connect harness lifecycle events to the same Rust core. Install the supermem binary first:

~~~sh
cargo install --path crates/super-mem-cli --locked
~~~

Packages and release binaries are not published yet. Use the files in this directory from a checkout.

| Harness | Automatic capture | Explicit access | Guide |
| --- | --- | --- | --- |
| Codex | Command hooks | MCP | [Codex](codex/README.md) |
| Claude Code | Command hooks | MCP | [Claude Code](claude/README.md) |
| OpenCode | TypeScript plugin | MCP | [OpenCode](opencode/README.md) |
| Pi | Native extension | CLI | [Pi](pi/README.md) |

## Shared behavior

- Captured text is sent through stdin, not command arguments.
- Automatic prompt, assistant, and compaction text is capped at 64 KiB.
- Stable content-sensitive keys deduplicate identical hook retries.
- Hook and plugin failures are fail-open.
- stdout is reserved for the host protocol; diagnostics go to stderr.
- Storage, scoping, ranking, and redaction remain in the Rust core.

For MCP, the trusted launch command pins the root, namespace, and optional workspace:

~~~sh
supermem mcp --root /path/to/repo --namespace default --workspace team-a
~~~

Model-facing schemas cannot select another root, repository, namespace, or workspace. The server rediscovers current Git state from the pinned root on each call.

See [Harness integrations](../docs/integrations.md) for the event matrix and [Security and privacy](../docs/privacy-and-threat-model.md) before enabling automatic capture.
