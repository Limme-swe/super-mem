# super-mem

[![CI](https://github.com/Limme-swe/super-mem/actions/workflows/ci.yml/badge.svg)](https://github.com/Limme-swe/super-mem/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Limme-swe/super-mem?display_name=tag)](https://github.com/Limme-swe/super-mem/releases/latest)
[![MSRV](https://img.shields.io/badge/rustc-1.88%2B-000000?logo=rust)](Cargo.toml)
[![License: MIT](https://img.shields.io/badge/license-MIT-2ea44f.svg)](LICENSE)

Local, Git-aware memory for coding agents.

super-mem records prompts, decisions, tool results, tests, and failed attempts in a local SQLite store. Recall is filtered by workspace, repository, and Git state before anything is ranked, so an old note from another branch is not treated as current fact.

**Status:** pre-1.0. The CLI and database schema may change between minor releases. Native release binaries are available for Linux, Windows, and both Intel and Apple Silicon Macs; crates.io and npm packages are not published yet.

[Installation](docs/installation.md) · [Architecture](docs/architecture.md) · [Search indexing](docs/search-indexing.md) · [Integrations](docs/integrations.md) · [Security](docs/privacy-and-threat-model.md) · [Evaluation](docs/evaluation.md) · [Contributing](CONTRIBUTING.md)

## What it does

- Keeps observable evidence with each memory instead of storing detached summaries.
- Separates successful procedures, failed attempts, decisions, facts, and open work.
- Tracks revisions, source events, historical links, corrections, retractions, and conflicting evidence without erasing history.
- Classifies repository memories as exact, compatible, stale, divergent, or unversioned.
- Builds a small, deterministic context packet under a fixed token budget.
- Builds bounded code-aware aliases and strict/loose lexical candidates, with optional background expansions and caller-supplied dense vectors.
- Serves concurrent MCP recalls through a bounded SQLite reader pool while serializing writes on one primary connection.
- Makes no model calls or downloads in the write and recall paths, and needs no hosted database or telemetry service.

The Rust core owns storage, scoping, retrieval, and context assembly. Harness adapters only translate lifecycle events and inject the resulting context.

## Install

Download the archive for your system from the [latest GitHub release](https://github.com/Limme-swe/super-mem/releases/latest), extract `supermem` (`supermem.exe` on Windows), and place it on `PATH`.

| System | Release target |
| --- | --- |
| Linux x86-64 | `x86_64-unknown-linux-musl` |
| Windows x86-64 | `x86_64-pc-windows-msvc` |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |

Every release includes `SHA256SUMS` and GitHub build-provenance attestations. See [Installation](docs/installation.md) for exact commands, default data locations, verification, unsigned-binary notices, and source builds.

Keep Git on `PATH` to enable repository identity, ancestry, changed-file capture, and file-freshness checks. The CLI still supports unscoped local memory when Git is unavailable.

## Quickstart

Create the local store:

~~~sh
supermem init
~~~

Record a repository decision:

~~~sh
supermem remember --kind decision --body "Use the workspace-level Cargo release profile" --file Cargo.toml --cwd .
~~~

Recall relevant context:

~~~sh
supermem recall --query "why is the release profile ignored?" --cwd . --token-budget 1200
~~~

`--file` is repeatable on `remember`, `recall`, and `checkpoint`; it records or checks repository-relative content hashes. Checkpoints also attempt to fingerprint the complete changed-file set by default and attach nothing if automatic capture is incomplete. Use `--no-auto-artifacts` to disable that capture.

Run the MCP server:

~~~sh
supermem mcp --root .
~~~

A generic stdio configuration looks like this:

~~~json
{
  "mcpServers": {
    "super-mem": {
      "command": "supermem",
      "args": ["mcp", "--root", "."]
    }
  }
}
~~~

The launch command pins the root, namespace, and optional workspace. Model tool arguments cannot replace those boundaries. `SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, and `SUPER_MEM_WORKSPACE` can provide the same store and scope to scoped CLI commands, hooks, and MCP; keep both processes configured alike.

## Integrations

Reference adapters are included in [adapters/](adapters/) in both the repository and each binary archive. Their npm packages are not published separately.

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
4. Exact, strict/loose lexical, code-alias, diagnostic, entity, artifact, and recency channels produce candidates; registered expansions and caller-supplied vectors can add optional semantic candidates.
5. Git applicability, importance, confidence, trust, feedback, and redundancy affect ranking.
6. Only selected immutable revisions receive full provenance hydration; their bodies are fetched and rendered as untrusted evidence under the requested budget.

Stale records require `--include-stale`. Descendant and diverged records are hidden unless recall uses `--include-divergent` or MCP sets `include_divergent`. A complete set of matching artifact hashes can keep a memory exact despite unrelated dirty files; incomplete automatic Git capture supplies no artifact evidence.

SQLite memory rows are canonical. FTS, deterministic aliases, fixed-width artifact fingerprints, and optional search projections are derived state. Rendered text and structured recall contain the same selected, safely truncated bodies, and the reported token estimate is computed from the final rendering. Default recall does not depend on network access or an embedding model; optional dense vectors are generated and supplied by the caller.

See [Search indexing](docs/search-indexing.md) for the background `pending`/`register` workflow, immutable profile activation and removal, artifact projection integrity checks, dense-vector scoring, rebuilds, snapshot behavior, and evaluation limits.

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

To build the CLI from source, install Rust 1.88 or newer, clone the repository, and run:

~~~sh
cargo install --path crates/super-mem-cli --locked
~~~

Run the full local checks with:

~~~sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
node scripts/validate-eval-fixture.mjs
node scripts/validate-retrieval-fixture.mjs
~~~

Changes to retrieval, scoping, snapshots, or performance need a focused regression test. Performance claims need a reproducible workload and hardware description.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the workflow, [SECURITY.md](SECURITY.md) for private vulnerability reporting, and [SUPPORT.md](SUPPORT.md) for usage questions.

## License

MIT
