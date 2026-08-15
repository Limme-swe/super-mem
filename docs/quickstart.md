# Five-minute quickstart

This path installs `supermem`, verifies that it works without touching an existing memory store, and records the first repository-scoped memory.

## 1. Install a verified release

The installer downloads the archive and `SHA256SUMS` from the same GitHub release, refuses a missing or mismatched checksum, and installs only the `supermem` executable.

### Linux or macOS

Download the installer, inspect it, then run it:

```sh
curl -fsSLO https://raw.githubusercontent.com/Limme-swe/super-mem/main/scripts/install.sh
less install.sh
sh install.sh
```

The default destination is `~/.local/bin/supermem`. Use `--install-dir` to choose another user-writable directory:

```sh
sh install.sh --install-dir "$HOME/bin"
```

### Windows

In PowerShell, download the installer, inspect it, then run it:

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/Limme-swe/super-mem/main/scripts/install.ps1 -OutFile install.ps1
Get-Content .\install.ps1
.\install.ps1
```

The default destination is `%LOCALAPPDATA%\Programs\super-mem\bin`. The installer adds that directory to the user `PATH`; open a new terminal if the current one does not see it.

Manual installation and release-attestation verification remain documented in [Installation](installation.md).

## 2. Confirm the binary and data path

From an extracted release archive or source checkout, run the non-destructive preflight:

```sh
python scripts/preflight.py --cwd .
```

It checks the executable, working directory, Git availability, and whether the configured database path can be used. It does not initialize or migrate a store unless `--doctor` is explicitly requested, and `doctor` itself remains observational.

Then initialize the normal local store:

```sh
supermem init
```

Run the isolated smoke test when you want an end-to-end check. It creates two temporary databases, exercises write, recall, export, import, status, and doctor, then deletes the temporary directory:

```sh
python scripts/smoke_install.py
```

## 3. Record a useful repository decision

Run this from a Git worktree:

```sh
supermem remember \
  --kind decision \
  --body "Use the workspace-level Cargo release profile" \
  --file Cargo.toml \
  --cwd .
```

Use `--body-stdin` instead of `--body` for long or sensitive content so it does not appear in a process listing:

```sh
printf '%s\n' 'Keep SQLite as canonical state; indexes must be rebuildable.' \
  | supermem remember --kind constraint --body-stdin --cwd .
```

## 4. Recall it

```sh
supermem recall \
  --query "where should the release profile live?" \
  --file Cargo.toml \
  --cwd . \
  --token-budget 800
```

Repository, workspace, namespace, lifecycle, time, and Git-applicability filters run before ranking. Stale or branch-divergent evidence is not silently presented as current.

## 5. Connect an agent

A generic MCP configuration is:

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

The launch root, namespace, and optional workspace are pinned for the server lifetime. Keep `SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, and `SUPER_MEM_WORKSPACE` identical between MCP and automatic-capture hooks or extensions.

Use [Harness integrations](integrations.md) for Codex, Claude Code, OpenCode, and Pi installation steps, then follow the [Integration checklist](integration-checklist.md) to verify the complete path.

## 6. Diagnose before guessing

Run `doctor` in the same environment that launches the agent:

```sh
supermem --json doctor --cwd /absolute/path/to/repository
```

For a report that is easier to attach to an issue, create a path-sanitized support file:

```sh
python scripts/support_bundle.py --cwd /absolute/path/to/repository --output super-mem-support.json
```

The collector does not copy memory rows, the database file, environment variables, or Git remotes. Review the generated JSON before sharing it.

## Next steps

- [Configuration reference](configuration.md)
- [CLI cookbook](cli-cookbook.md)
- [Troubleshooting](troubleshooting.md)
- [Backup, upgrade, and uninstall](data-lifecycle.md)
- [Operator security checklist](operator-security-checklist.md)
