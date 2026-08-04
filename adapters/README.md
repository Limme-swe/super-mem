# Harness adapters

All adapters require the `supermem` binary to be installed and available on
`PATH`. Until release binaries are published, build it from the repository
root with `cargo install --path crates/super-mem-cli --locked`.

These adapters keep automatic memory capture and recall close to each harness's
native lifecycle while sharing the `supermem` Rust binary.

| Harness | Automatic path | Explicit tools |
| --- | --- | --- |
| Codex | command hooks | MCP over stdio |
| Claude Code | command hooks | MCP over stdio |
| OpenCode | TypeScript plugin events | MCP over stdio |
| Pi | native TypeScript extension | native extension commands |

The MVP starts a short-lived `supermem` process for each hook or plugin event.
The CLI boundary is deliberately stable: a future resident daemon can be added
behind the same commands (Unix socket on Unix, named pipe on Windows) without
changing any harness configuration. Adapters must fail open: memory being
unavailable must never prevent the coding agent from continuing.

`supermem mcp` reserves stdout for JSON-RPC. Hooks likewise emit only the JSON
object required by the host on stdout; diagnostics go to stderr.

MCP isolation is pinned by the trusted launch command, for example
`supermem mcp --root /path/to/repo --namespace default --workspace team-a`.
The server rediscovers Git state from that root on every call. Model-facing
schemas expose only an optional session ID; they cannot select a namespace,
working directory, repository, or workspace.
