# super-mem

[![CI](https://github.com/Limme-swe/super-mem/actions/workflows/ci.yml/badge.svg)](https://github.com/Limme-swe/super-mem/actions/workflows/ci.yml)
[![Comfort checks](https://github.com/Limme-swe/super-mem/actions/workflows/comfort.yml/badge.svg)](https://github.com/Limme-swe/super-mem/actions/workflows/comfort.yml)
[![Release](https://img.shields.io/github/v/release/Limme-swe/super-mem?display_name=tag)](https://github.com/Limme-swe/super-mem/releases/latest)
[![MSRV](https://img.shields.io/badge/rustc-1.88%2B-000000?logo=rust)](Cargo.toml)
[![License: MIT](https://img.shields.io/badge/license-MIT-2ea44f.svg)](LICENSE)

Local, Git-aware memory for coding agents.

super-mem records prompts, decisions, tool results, tests, and failed attempts in a local SQLite store. Recall is filtered by namespace, workspace, repository, lifecycle, time, and Git applicability before anything is ranked, so an old note from another branch is not silently treated as current fact.

**Status:** pre-1.0. The CLI and database schema may change between minor releases. Native release binaries are available for Linux, Windows, and both Intel and Apple Silicon Macs; crates.io and npm packages are not published yet.

[Five-minute quickstart](docs/quickstart.md) · [Installation](docs/installation.md) · [Configuration](docs/configuration.md) · [CLI cookbook](docs/cli-cookbook.md) · [Integrations](docs/integrations.md) · [Integration checklist](docs/integration-checklist.md) · [Troubleshooting](docs/troubleshooting.md) · [Data lifecycle](docs/data-lifecycle.md) · [Security](docs/privacy-and-threat-model.md) · [Architecture](docs/architecture.md) · [Search indexing](docs/search-indexing.md) · [Evaluation](docs/evaluation.md) · [Development](docs/development-workflow.md) · [Contributing](CONTRIBUTING.md)

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

### Verified installer

Download the installer, inspect it, then run it. The installer retrieves the archive and `SHA256SUMS` from the same release, requires an exact checksum match, stages the executable, and verifies the installed version.

Linux or macOS:

```sh
curl -fsSLO https://raw.githubusercontent.com/Limme-swe/super-mem/main/scripts/install.sh
less install.sh
sh install.sh
```

The default destination is `~/.local/bin/supermem`. Choose another user-writable directory with `--install-dir`.

Windows PowerShell:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/Limme-swe/super-mem/main/scripts/install.ps1 -OutFile install.ps1
Get-Content .\install.ps1
.\install.ps1
```

The default destination is `%LOCALAPPDATA%\Programs\super-mem\bin`. The Windows installer adds it to the user `PATH`; open a new terminal when necessary.

### Manual release installation

Download the archive for your system from the [latest GitHub release](https://github.com/Limme-swe/super-mem/releases/latest), verify it, extract `supermem` (`supermem.exe` on Windows), and place it on `PATH`.

| System | Release target |
| --- | --- |
| Linux x86-64 | `x86_64-unknown-linux-musl` |
| Windows x86-64 | `x86_64-pc-windows-msvc` |
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |

Every release includes `SHA256SUMS` and GitHub build-provenance attestations. See [Installation](docs/installation.md) for exact manual commands, default data locations, verification, unsigned-binary notices, and source builds.

Keep Git on `PATH` to enable repository identity, ancestry, changed-file capture, and file-freshness checks. The CLI still supports unscoped local memory when Git is unavailable.

### Verify the installation

From a source checkout or extracted release archive, run the non-destructive environment check:

```sh
python scripts/preflight.py --cwd .
```

Then exercise the complete lifecycle against temporary databases only:

```sh
python scripts/smoke_install.py
```

The smoke test runs version, init, remember, recall, export, import, status, and doctor without reusing `SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, or `SUPER_MEM_WORKSPACE`.

## Quickstart

The [five-minute quickstart](docs/quickstart.md) gives the full install-to-agent path. The core CLI flow is:

```sh
supermem init
```

Record a repository decision:

```sh
supermem remember \
  --kind decision \
  --body "Use the workspace-level Cargo release profile" \
  --file Cargo.toml \
  --cwd .
```

For long or sensitive text, prefer stdin so content does not appear in a process listing:

```sh
printf '%s\n' 'Keep SQLite canonical; every search index must be rebuildable.' \
  | supermem remember --kind constraint --body-stdin --cwd .
```

Recall relevant context:

```sh
supermem recall \
  --query "why is the release profile ignored?" \
  --file Cargo.toml \
  --cwd . \
  --token-budget 1200
```

`--file` is repeatable on `remember`, `recall`, and `checkpoint`; it records or checks repository-relative content hashes. Checkpoints also attempt to fingerprint the complete changed-file set by default and attach nothing if automatic capture is incomplete. Use `--no-auto-artifacts` to disable that capture.

Run the MCP server:

```sh
supermem mcp --root .
```

A generic stdio configuration looks like this:

```json
{
  "mcpServers": {
    "super-mem": {
      "command": "supermem",
      "args": ["mcp", "--root", "."]
    }
  }
}
```

The launch command pins the root, namespace, and optional workspace. Model tool arguments cannot replace those boundaries. `SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, and `SUPER_MEM_WORKSPACE` can provide the same store and hard scope to scoped CLI commands, hooks, and MCP; keep every process configured alike. See [Configuration](docs/configuration.md) for precedence and platform defaults.

## Integrations

Reference adapters are included in [adapters/](adapters/) in both the repository and each binary archive. Their npm packages are not published separately.

| Harness | Explicit access | Automatic capture | Included adapter |
| --- | --- | --- | --- |
| Codex | MCP | Command hooks | Plugin manifest and hook configuration |
| Claude Code | MCP | Command hooks | Project MCP and hook configuration |
| OpenCode | MCP | TypeScript plugin | Source plugin and project configuration |
| Pi | CLI management | Native extension | Source extension package |
| Generic client | MCP | Host-dependent | Stdio configuration |

Automatic capture is fail-open: a memory error must not stop the coding session. A session continuing normally therefore does not prove that capture succeeded. See [Harness integrations](docs/integrations.md) for host-specific installation, then use the [integration checklist](docs/integration-checklist.md) to verify CLI storage, MCP access, lifecycle capture, compaction, restart behavior, and hard-scope consistency separately.

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

Database status, import/export, and physical purge remain CLI operations. The [CLI cookbook](docs/cli-cookbook.md) covers the complete command surface with copy-ready examples.

## Diagnose problems

Start outside the harness so installation, data-path, Git, and adapter failures are not conflated:

```sh
python scripts/preflight.py --cwd /absolute/path/to/repository --doctor
```

Run the explicit observational health check in the same environment that launches the agent:

```sh
supermem --json doctor --cwd /absolute/path/to/repository
```

`doctor` requires an initialized store and never creates, migrates, checkpoints, recovers, or changes its journal mode. It pins the audited database identity, checks writer access with native file locks, and inspects only a stable copy. Unix uses a mode-`0600` temporary file; Windows uses an in-memory copy capped at 512 MiB so database contents never inherit a temporary-directory ACL.

Copying and SQLite work share a five-second deadline, SQLite value sizes are capped, and integrity includes canonical cross-table relationships in addition to quick-check, foreign-key, and exact-schema checks. Live WAL state and rollback journals are reported without letting SQLite open the source because even a read-only SQLite connection can rewrite shared-memory state. File aliases and Unix permissions are checked, Git subprocesses have a two-second aggregate deadline and bounded output, and scope environment values are redacted. A machine-local identity digest makes path exchange observations correlatable without emitting raw device/file IDs. Required-check failure produces a nonzero exit.

The JSON report still contains machine-local paths and a credential-free repository probe. For a smaller path-sanitized issue attachment, generate and review:

```sh
python scripts/support_bundle.py \
  --cwd /absolute/path/to/repository \
  --output super-mem-support.json
```

The support collector does not copy memory rows, the database file, environment variables, or Git remotes. Read [Troubleshooting](docs/troubleshooting.md) before changing retrieval or weakening a path-safety check.

The CLI exposes memory history as JSON:

```sh
supermem --json inspect MEMORY_ID --history
```

## Data and safety

The default store is local, plaintext SQLite. New files use restrictive permissions where the platform supports them. Treat the store like source code, shell history, or terminal transcripts.

- With the default configuration, common credential patterns are redacted before storage. Redaction is not a guarantee.
- Recalled content is labeled as untrusted evidence, never promoted to instructions.
- Namespace, workspace, repository, lifecycle, time, and Git-applicability filters run before ranking.
- Automatic capture is capped and uses stdin rather than command arguments for captured text.
- Full snapshots are integrity-checked. Import is atomic and requires an otherwise empty store.

Keep the database outside the Git worktree unless there is a specific reason not to. Repository-local paths have additional platform and link-safety restrictions. Read [Security and privacy](docs/privacy-and-threat-model.md) and the [operator security checklist](docs/operator-security-checklist.md) before enabling automatic capture.

Create a logical backup and test a restore in a separate store:

```sh
supermem export --output memory.jsonl
supermem --db restored.sqlite3 import memory.jsonl
supermem --db restored.sqlite3 --json status
```

Use [Backup, upgrade, restore, and uninstall](docs/data-lifecycle.md) for safe upgrades, release checks, downgrade cautions, retention, and confirmed data deletion. The supplied uninstallers remove the executable while preserving memories by default:

```sh
sh scripts/uninstall.sh
```

```powershell
.\scripts\uninstall.ps1
```

## Development

To build the CLI from source, install Rust 1.88 or newer, clone the repository, and run:

```sh
cargo install --path crates/super-mem-cli --locked
```

Discover the cross-platform local check targets:

```sh
python scripts/dev.py --list
```

Run the fast review loop:

```sh
python scripts/dev.py quick
```

Run the complete local validation before a pull request:

```sh
python scripts/dev.py full
```

The full target covers formatting, Clippy with warnings denied, Rust tests, Python helper tests, documentation links, evaluation and retrieval fixtures, and package construction. Use `--dry-run` to inspect commands and `--keep-going` to collect all failures.

Changes to retrieval, scoping, snapshots, or performance need a focused regression test. Performance claims need a reproducible workload and hardware description. See [Development workflow](docs/development-workflow.md), [CONTRIBUTING.md](CONTRIBUTING.md), [SECURITY.md](SECURITY.md), and [SUPPORT.md](SUPPORT.md).

## License

MIT
