# Configuration reference

`supermem` has deliberately few global settings. Storage and hard isolation are selected explicitly; repository and Git state are discovered from the working directory unless the caller provides overrides.

## Global database selection

Every CLI command accepts the global `--db PATH` option. The equivalent environment variable is `SUPER_MEM_DB`.

```sh
supermem --db "$HOME/.local/share/super-mem/work.sqlite3" init
```

```sh
export SUPER_MEM_DB="$HOME/.local/share/super-mem/work.sqlite3"
supermem init
```

A command-line value wins over the environment. No repository `.env` file is loaded automatically.

Default paths are:

| System | Database path |
| --- | --- |
| Linux | `$XDG_DATA_HOME/super-mem/memory.sqlite3`, or `~/.local/share/super-mem/memory.sqlite3` |
| macOS | `~/Library/Application Support/super-mem/memory.sqlite3` |
| Windows | `%LOCALAPPDATA%\super-mem\memory.sqlite3` |

Keep the canonical database outside the Git worktree unless a specific deployment requires otherwise. Repository-local paths receive stricter link and file-identity checks. On Windows, use a local NTFS or ReFS volume rather than a network share.

## Namespace and workspace isolation

Most scoped commands accept:

- `--namespace NAME`, or `SUPER_MEM_NAMESPACE`; default: `default`.
- `--workspace NAME`, or `SUPER_MEM_WORKSPACE`; optional.

Namespace and workspace are hard filters, not ranking hints. A record written in one namespace or workspace is not returned from another.

A practical split between personal and work contexts is:

```sh
export SUPER_MEM_NAMESPACE=work
export SUPER_MEM_WORKSPACE=payments-platform
supermem recall --query "release procedure" --cwd .
```

Use stable, non-secret identifiers. Do not put credentials, access tokens, or customer data in namespace or workspace names.

## Repository and Git scope

Scoped commands discover repository identity from `--cwd`. They also accept explicit provenance overrides:

| Option | Purpose |
| --- | --- |
| `--repo-id` | Explicit repository identity instead of discovery. |
| `--branch` | Override the discovered branch. |
| `--head` | Override the discovered commit. |
| `--remote` | Override the discovered remote identity. |
| `--session` | Attach a harness session identity as provenance. |
| `--harness` | Record the calling harness as provenance. |

Prefer discovery from a real working tree. Overrides are primarily for controlled integrations and tests; inconsistent values can make evidence appear stale, divergent, or unversioned.

Git is optional for unscoped local memory, but repository-aware applicability depends on it. Keep the intended `git` executable on `PATH` for commit ancestry, changed-file capture, dirty-worktree classification, and artifact freshness.

## File applicability evidence

`remember`, `recall`, and `checkpoint` accept repeatable `--file` values. Paths are repository-relative and are fingerprinted as evidence of applicability.

```sh
supermem remember \
  --kind procedure \
  --body "Regenerate the lockfile after changing workspace dependencies" \
  --file Cargo.toml \
  --file Cargo.lock \
  --cwd .
```

Checkpoints also attempt to capture the complete bounded changed-file set. If automatic capture is incomplete, no inferred artifact set is attached. Disable the attempt with `--no-auto-artifacts` and supply explicit files when needed.

## MCP and hook consistency

The MCP server pins its root, namespace, and optional workspace at launch:

```sh
SUPER_MEM_NAMESPACE=work \
SUPER_MEM_WORKSPACE=payments-platform \
supermem mcp --root /absolute/path/to/repository
```

Automatic-capture hooks and extensions must use the same database, namespace, and workspace. A common failure mode is launching MCP from one shell and the harness from another shell with different environment values.

Use this diagnostic in the exact harness environment:

```sh
python scripts/preflight.py --cwd /absolute/path/to/repository --doctor
```

## JSON output

`--json` is a global option for ordinary CLI commands:

```sh
supermem --json status
supermem --json inspect MEMORY_ID --history
supermem --json doctor --cwd .
```

`recall` also has `--format json` for its result format. Do not assume human-readable output is a stable machine interface.

## Sensitive input

Prefer stdin forms when content may be long or sensitive:

```sh
supermem remember --kind fact --body-stdin --cwd . < note.txt
supermem observe --kind tool_result --content-stdin --cwd . < result.txt
supermem recall --query-stdin --cwd . < query.txt
supermem checkpoint --summary-stdin --cwd . < summary.txt
```

The default redactor catches common credential patterns, but it is not a guarantee. Treat the local plaintext database like source code, shell history, or terminal transcripts. See [Security and privacy](privacy-and-threat-model.md) and the [Operator security checklist](operator-security-checklist.md).
