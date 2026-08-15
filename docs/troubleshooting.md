# Troubleshooting

Start with the smallest command that can distinguish installation, storage, Git, and harness problems:

```sh
python scripts/preflight.py --cwd /absolute/path/to/repository --doctor
```

For issue reports, generate a bounded path-sanitized file and review it before sharing:

```sh
python scripts/support_bundle.py \
  --cwd /absolute/path/to/repository \
  --output super-mem-support.json
```

## `supermem: command not found`

Confirm the installation directory and current `PATH`:

```sh
ls -l "$HOME/.local/bin/supermem"
printf '%s\n' "$PATH"
```

On Windows:

```powershell
Get-Command supermem -ErrorAction SilentlyContinue
[Environment]::GetEnvironmentVariable('Path', 'User')
```

The POSIX installer does not edit shell startup files. Add `~/.local/bin` to `PATH` or choose an existing directory with `--install-dir`. The Windows installer updates the user `PATH`; open a new terminal after installation.

## Unsupported platform or architecture

Published binaries currently cover Linux x86-64, Windows x86-64, macOS Apple Silicon, and macOS Intel. Build from source for another supported Rust target, or use a supported machine. Do not force `--target` to an incompatible archive.

## Checksum verification failed

Do not bypass the check. Delete the downloaded files and retry. A mismatch can mean an interrupted download, a proxy or cache serving inconsistent content, or a release asset that does not match `SHA256SUMS`.

Manual verification is in [Installation](installation.md). GitHub build provenance can be checked separately with `gh attestation verify`.

## macOS blocks the binary

The release is not notarized with an Apple Developer ID. Verify the checksum and provenance first, then use **System Settings → Privacy & Security → Open Anyway** if the verified binary is trusted.

## Windows SmartScreen warns

The executable is not Authenticode-signed. Verify `SHA256SUMS` and GitHub provenance before approving it. Do not download an executable from a third-party mirror.

## `doctor` reports a missing or old store

This is intentional: `doctor` is observational and does not initialize or migrate. Run the normal initialization path first:

```sh
supermem init
```

After an upgrade, export a backup before opening the store with the new binary. See [Backup, upgrade, and uninstall](data-lifecycle.md).

## Database is locked or sidecars are live

Stop the MCP server, hooks, extensions, and any other `supermem` process that uses the database. Then rerun:

```sh
supermem --json doctor --cwd .
```

Do not copy only `memory.sqlite3` while a WAL or rollback journal is active. Use `supermem export` for a logical backup. `doctor` intentionally refuses to recover or open live journal state.

## Adapter appears silent

Check the complete path rather than only the adapter file:

1. `supermem --version` succeeds in the harness environment.
2. `supermem init` has initialized the same database.
3. MCP and automatic capture use the same `SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, and `SUPER_MEM_WORKSPACE`.
4. The configured MCP root is the intended repository.
5. Hook or extension timeouts exceed the SQLite busy timeout.
6. `supermem --json doctor --cwd ...` succeeds from the same environment.

Reference adapters are fail-open. A coding session continuing normally does not prove that capture succeeded. Use the [Integration checklist](integration-checklist.md).

## Recall returns nothing

Common causes are:

- a different namespace, workspace, database, or repository;
- a query outside the recorded evidence;
- a stale, descendant, or branch-divergent record excluded by default;
- a retracted or superseded memory;
- a file fingerprint that no longer matches;
- automatic capture that never reached the store.

Inspect status and history, then test a direct query:

```sh
supermem --json status
supermem recall --query "exact phrase from the memory" --cwd . --format json
supermem --json inspect MEMORY_ID --history
```

Use `--include-stale`, `--include-divergent`, or `--include-superseded` only when that evidence is intentionally wanted.

## Git-aware results are unavailable

```sh
git --version
git -C /absolute/path/to/repository rev-parse --is-inside-work-tree
```

Git is optional for unscoped storage, but repository identity, ancestry, dirty-state classification, changed-file capture, and artifact freshness depend on it. Ensure the harness sees the intended `git` executable on `PATH`.

## Permission or path-safety failure

Keep the database in the platform user data directory or another private local directory. Avoid symlinked, multiply linked, world-readable, or repository-controlled database paths. On Windows, network shares are unsupported for the canonical SQLite store.

Do not fix a safety failure by making the database broadly writable. Read [Security and privacy](privacy-and-threat-model.md) and the [Operator security checklist](operator-security-checklist.md).

## Import fails

Import expects an otherwise empty destination store and buffers the snapshot. Use a new database path:

```sh
supermem --db restored.sqlite3 import memory.jsonl
```

Keep the original database and snapshot until the restored store passes `status`, `doctor`, and a representative recall.

## Still unresolved

Include:

- the exact command and exit code;
- operating system and architecture;
- `supermem --version`;
- whether the database is default or custom;
- whether the issue occurs outside the harness;
- the reviewed support report.

Never attach the SQLite database or raw memory export to a public issue unless the contents were intentionally reviewed and approved for disclosure.
