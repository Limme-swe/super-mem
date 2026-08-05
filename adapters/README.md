# Harness adapters

The reference adapters connect harness lifecycle events to the same Rust core. Install the `supermem` binary from the [latest release](https://github.com/Limme-swe/super-mem/releases/latest) first. A source build is also available:

~~~sh
cargo install --path crates/super-mem-cli --locked
~~~

The adapter npm packages are not published separately. Use the files in this directory from the source tree or an extracted binary release archive.

| Harness | Automatic capture | Explicit access | Guide |
| --- | --- | --- | --- |
| Codex | Command hooks | MCP | [Codex](codex/README.md) |
| Claude Code | Command hooks | MCP | [Claude Code](claude/README.md) |
| OpenCode | TypeScript plugin | MCP | [OpenCode](opencode/README.md) |
| Pi | Native extension | CLI | [Pi](pi/README.md) |

## Shared behavior

- Captured text is sent through stdin, not command arguments.
- Automatic prompt, assistant, and compaction text is capped at 64 KiB.
- Exposed tool, command, test, and file outcomes are recorded; memory-management tools are excluded.
- Checkpoints attach a complete bounded set of changed-file hashes when Git can supply one; partial automatic capture is discarded.
- Stable content-sensitive keys deduplicate identical hook retries.
- Hook and plugin failures are fail-open.
- stdout is reserved for the host protocol; diagnostics go to stderr.
- Storage, scoping, ranking, and redaction remain in the Rust core.

For MCP, the trusted launch command pins the root, namespace, and optional workspace:

~~~sh
supermem mcp --root .
~~~

Model-facing schemas cannot select another root, repository, namespace, or workspace. The server rediscovers current Git state from the pinned root on each call.

`SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, and `SUPER_MEM_WORKSPACE` configure the store and scope used by CLI commands, hooks, and MCP. Set them consistently for the adapter and MCP processes. MCP `memory_manage` supports current inspection, immutable revision, event, link, and feedback history, and retraction.

See [Harness integrations](../docs/integrations.md) for the event matrix and [Security and privacy](../docs/privacy-and-threat-model.md) before enabling automatic capture.
