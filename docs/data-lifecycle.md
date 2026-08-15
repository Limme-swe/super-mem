# Backup, upgrade, restore, and uninstall

The canonical store is local plaintext SQLite. Treat changes to it like changes to source code or shell history: take a logical backup before an upgrade, test restores to a separate path, and preserve the original until verification succeeds.

## Create a logical backup

Stop or quiesce automation when practical, then export the canonical JSONL representation:

```sh
supermem export --output memory-$(date +%Y%m%d).jsonl
```

On PowerShell:

```powershell
$Date = Get-Date -Format yyyyMMdd
supermem export --output "memory-$Date.jsonl"
```

Protect the export like the database. It can contain prompts, source fragments, decisions, tool output, and other private repository context.

A filesystem copy of only `memory.sqlite3` is not a safe substitute while WAL or rollback-journal state is active. Use export for portable backups.

## Verify a backup by restoring it

Import into a new, otherwise empty database:

```sh
supermem --db ./restore-check.sqlite3 import memory-20260815.jsonl
supermem --db ./restore-check.sqlite3 status
supermem --db ./restore-check.sqlite3 --json doctor --cwd .
supermem --db ./restore-check.sqlite3 recall --query "representative decision" --cwd .
```

Do not point the restore test at the canonical path. Keep the original store and export until representative recall and history checks pass.

The included install smoke test performs an isolated export/import cycle with synthetic data:

```sh
python scripts/smoke_install.py
```

## Upgrade the binary

1. Record the current version.
2. Export the store.
3. Verify the new release checksum and provenance.
4. Replace the executable.
5. Open the store through the normal CLI path.
6. Run status, doctor, and a representative recall.

```sh
supermem --version
supermem export --output pre-upgrade.jsonl
sh scripts/install.sh
supermem --version
supermem init
supermem --json status
supermem --json doctor --cwd .
```

On Windows, use `scripts/install.ps1` instead. Installers replace only the executable; they do not delete or rewrite the database directly.

`supermem init` is the normal initialization/open path. `doctor` intentionally does not migrate an old schema. Keep the pre-upgrade export until the new binary has been exercised by the intended harness.

## Check for a newer release

```sh
python scripts/check_update.py
```

Machine-readable output:

```sh
python scripts/check_update.py --json
```

Use `--require-current` in maintenance automation when an available update should produce exit code `10` without treating network or parsing failures as the same condition.

## Downgrade cautiously

Pre-1.0 CLI and schema compatibility can change between minor releases. Do not assume an older binary can open a store already migrated by a newer binary. Restore the export into a separate database using the intended version, then validate before changing the canonical path.

## Uninstall the binary but preserve memories

Linux or macOS:

```sh
sh scripts/uninstall.sh
```

Windows PowerShell:

```powershell
.\scripts\uninstall.ps1
```

Both uninstallers preserve the default memory data by default. A custom database selected with `--db` or `SUPER_MEM_DB` is never deleted automatically.

## Permanently delete the default data

Stop every process using the store and take any required export first.

Linux or macOS:

```sh
sh scripts/uninstall.sh --purge-data --yes
```

Windows PowerShell:

```powershell
.\scripts\uninstall.ps1 -PurgeData -Yes
```

Preview without deleting:

```sh
sh scripts/uninstall.sh --purge-data --yes --dry-run
```

```powershell
.\scripts\uninstall.ps1 -PurgeData -Yes -DryRun
```

The CLI also exposes explicit database deletion for the currently selected path:

```sh
supermem purge --yes
```

`purge` removes the selected database and sidecars. Verify `--db` or `SUPER_MEM_DB` before confirming.

## Retention checklist

- Keep at least one tested export before upgrades or path moves.
- Store backups outside the repository and outside publicly synchronized folders.
- Encrypt backups when the host or storage boundary requires it.
- Delete obsolete exports intentionally; uninstalling the executable does not remove them.
- Never commit the database, WAL, rollback journal, or export to Git.
