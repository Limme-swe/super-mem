# super-mem

[![CI](https://github.com/Limme-swe/super-mem/actions/workflows/ci.yml/badge.svg)](https://github.com/Limme-swe/super-mem/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/rustc-1.88%2B-000000?logo=rust)](Cargo.toml)
[![License: MIT](https://img.shields.io/badge/license-MIT-2ea44f.svg)](LICENSE)

Local, Git-aware memory for coding agents.

super-mem records prompts, decisions, tool results, tests, and failed attempts in a local SQLite store. Recall is filtered by workspace, repository, and Git state before anything is ranked, so an old note from another branch is not treated as current fact.

**Status:** pre-1.0. The CLI and database schema may change before the first stable release. Packages and release binaries are not published yet.

[Architecture](docs/architecture.md) · [Integrations](docs/integrations.md) · [Security](docs/privacy-and-threat-model.md) · [Evaluation](docs/evaluation.md) · [Contributing](CONTRIBUTING.md)

## What it does

- Keeps observable evidence with each memory instead of storing detached summaries.
- Separates successful procedures, failed attempts, decisions, facts, and open work.
- Tracks revisions, source events, historical links, corrections, retractions, and conflicting evidence without erasing history.
- Classifies repository memories as exact, compatible, stale, divergent, or unversioned.
- Builds a small, deterministic context packet under a fixed token budget.
- Runs locally without an embedding model, hosted database, or telemetry service.

The Rust core owns storage, scoping, retrieval, and context assembly. Harness adapters only translate lifecycle events and inject the resulting context.

## Install from source

The workspace requires Rust 1.88 or newer.

~~~sh
git clone https://github.com/Limme-swe/super-mem.git
cd super-mem
cargo install --path crates/super-mem-cli --locked
supermem --version
~~~

## Quickstart

Create the local store:

~~~sh
supermem init
~~~

Record a repository decision:

~~~sh
supermem remember \
  --kind decision \
  --body "Use the workspace-level Cargo release profile" \
  --file Cargo.toml \
  --cwd .
~~~

Recall relevant context:

~~~sh
supermem recall \
  --query "why is the release profile ignored?" \
  --cwd . \
  --token-budget 1200
~~~

`--file` is repeatable on `remember`, `recall`, and `checkpoint`; it records or checks repository-relative content hashes. Checkpoints also attempt to fingerprint the complete changed-file set by default and attach nothing if automatic capture is incomplete. Use `--no-auto-artifacts` to disable that capture.

Run the MCP server:

~~~sh
supermem mcp --root /absolute/path/to/repo --namespace default
~~~

A generic stdio configuration looks like this:

~~~json
{
  "mcpServers": {
    "super-mem": {
      "command": "supermem",
      "args": ["mcp", "--root", "/absolute/path/to/repo", "--namespace", "default"]
    }
  }
}
~~~

The launch command pins the root, namespace, and optional workspace. Model tool arguments cannot replace those boundaries. `SUPER_MEM_NAMESPACE` and `SUPER_MEM_WORKSPACE` can provide the same scope to scoped CLI commands, hooks, and MCP; keep both processes configured alike.

## Integrations

Reference adapters are included in [adapters/](adapters/). They currently install from a checkout; their npm packages are not published.

| Harness | Explicit access | Automatic capture | Included adapter |
| --- | --- | --- | --- |
| Codex | MCP | Command hooks | Plugin manifest and hook configuration |
| Claude Code | MCP | Command hooks | Project MCP and hook configuration |
| OpenCode | MCP | TypeScript plugin | Source plugin and project configuration |
| Pi | CLI management | Native extension | Source extension package |
| Generic client | MCP | Host-dependent | Stdio configuration |

Automatic capture is fail-open: a memory error must not stop the coding session. See [Harness integrations](docs/integrations.md) for installation details and the events captured by each adapter.

## How recall works

1. Harness events and explicit records are validated, redacted, and appended to SQLite.
2. Queryable records keep their source events, outcome, scope, and revision history.
3. Hard namespace, workspace, repository, lifecycle, and time filters run before channel limits.
4. Exact, lexical, diagnostic, entity, artifact, and recency channels produce candidates.
5. Git applicability, feedback, evidence quality, and redundancy affect ranking.
6. Selected records are rendered as untrusted evidence under the requested budget.

Stale records require `--include-stale`. Descendant and diverged records are hidden unless recall uses `--include-divergent` or MCP sets `include_divergent`. A complete set of matching artifact hashes can keep a memory exact despite unrelated dirty files; incomplete automatic Git capture supplies no artifact evidence.

SQLite rows are canonical. FTS and lookup indexes are rebuildable projections. Rendered text and structured recall contain the same selected, safely truncated bodies, and the reported token estimate is computed from the final rendering. Recall does not depend on network access or an embedding model.

## MCP tools

The model-facing surface has four tools:

| Tool | Purpose |
| --- | --- |
| **memory_context** | Recall a scoped context packet for the current task. |
| **memory_record** | Store a typed memory, observation, or checkpoint. |
| **memory_feedback** | Attach an observed result or judgment to a memory. |
| **memory_manage** | Inspect a memory, load its revision, event, link, and feedback history, or retract it. |

Database status, import/export, and physical purge remain CLI operations.

The CLI exposes the same history as JSON:

~~~sh
supermem --json inspect MEMORY_ID --history
~~~

## Data and safety

The default store is local, plaintext SQLite. New files use restrictive permissions where the platform supports them. Treat the store like source code or terminal history.

- With the default configuration, common credential patterns are redacted before storage. Redaction is not a guarantee.
- Recalled content is labeled as untrusted evidence, never promoted to instructions.
- Repository and workspace filters are applied before ranking.
- Automatic capture is capped and uses stdin rather than command arguments for captured text.
- Full snapshots are integrity-checked. Import is atomic and requires an otherwise empty store.

Keep the database outside the Git worktree unless there is a specific reason not to. Repository-local paths have additional platform and link-safety restrictions. Read [Security and privacy](docs/privacy-and-threat-model.md) before enabling automatic capture.

Create or restore a snapshot with:

~~~sh
supermem export --output memory.jsonl
supermem import memory.jsonl
~~~

## Development

Run the full local checks with:

~~~sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
node scripts/validate-eval-fixture.mjs
~~~

Changes to retrieval, scoping, snapshots, or performance need a focused regression test. Performance claims need a reproducible workload and hardware description.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow, [SECURITY.md](SECURITY.md) for private vulnerability reporting, and [SUPPORT.md](SUPPORT.md) for usage questions.

## License

MIT
