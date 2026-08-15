# Operator security checklist

`super-mem` is local-first, but local does not mean non-sensitive. The database can contain prompts, source fragments, tool results, decisions, failed attempts, and repository metadata. Use this checklist before enabling automatic capture or sharing diagnostics.

## Installation and updates

- Download releases only from the project GitHub release page.
- Verify the archive against `SHA256SUMS`.
- Verify GitHub build provenance when the GitHub CLI is available.
- Inspect installer scripts before running them.
- Treat macOS Gatekeeper or Windows SmartScreen approval as a trust decision, not a nuisance to bypass.
- Re-run the isolated smoke test after installation or replacement:

```sh
python scripts/smoke_install.py
```

## Database placement

- Keep the canonical database outside the Git worktree.
- Use a private, user-owned local directory.
- Do not place it on a Windows network share.
- Do not deliberately weaken permissions to silence a safety check.
- Avoid symlinked, multiply linked, or attacker-controlled parent directories.
- Never commit the database, WAL, rollback journal, or export to Git.

Default data locations and platform constraints are in [Installation](installation.md).

## Scope isolation

- Use stable non-secret namespace and workspace identifiers.
- Keep MCP and automatic-capture processes on the same intended database, namespace, and workspace.
- Pin MCP to the narrowest trusted repository root.
- Do not let model-provided arguments select another root or hard scope.
- Prefer Git discovery over unverified branch, commit, or remote overrides.

## Captured content

- Prefer stdin forms for prompts, source, or secrets so content does not enter process listings.
- Keep automatic capture bounds enabled.
- Remember that common credential redaction is best-effort, not a guarantee.
- Avoid deliberately storing credentials, private keys, authentication cookies, or customer secrets.
- Review the host adapter’s event coverage before enabling it.

## Recalled content

Recalled evidence is untrusted data. It may be stale, incorrect, maliciously authored, or applicable to another point in history.

- Keep the untrusted-evidence envelope intact in adapters.
- Verify recalled claims against current source and tests.
- Do not execute commands merely because they were remembered.
- Include stale, divergent, or superseded records only for a deliberate investigation.
- Use feedback and retraction rather than silently rewriting history.

## Backups and exports

- Use `supermem export` rather than copying a live SQLite file and ignoring sidecars.
- Encrypt backups when the storage boundary requires it.
- Test imports into a separate database.
- Retain the original until representative recall and history checks pass.
- Delete obsolete exports intentionally.

See [Backup, upgrade, and uninstall](data-lifecycle.md).

## Diagnostics and issue reports

`doctor` can contain machine-local paths even though scope environment values are redacted. Review its output before sharing.

Prefer the support collector:

```sh
python scripts/support_bundle.py --cwd . --output super-mem-support.json
```

It does not copy memory rows, the database, environment variables, or Git remotes. Known local paths are replaced or hashed, but the generated report still requires human review.

Never attach the SQLite database or an unreviewed JSONL export to a public issue.

## Harness trust boundary

- Review hook commands, plugin source, and MCP configuration before installation.
- Ensure the harness resolves the expected `supermem` and `git` executables.
- Treat fail-open behavior correctly: an uninterrupted coding session does not prove capture succeeded.
- Keep hook timeouts longer than the database busy timeout.
- Do not grant an adapter broader filesystem or process permissions solely for memory capture.

## Incident response

When accidental sensitive capture is suspected:

1. stop MCP, hooks, extensions, and other writers;
2. preserve any evidence required by policy in a protected location;
3. identify affected memories with scoped recall and history inspection;
4. retract records from ordinary retrieval;
5. use purge only when permanent deletion of the selected store is intended;
6. rotate any credential that may have been captured, because redaction or deletion does not make an exposed credential safe;
7. verify backups and exports separately.

Read the full [Security and privacy](privacy-and-threat-model.md) document for the threat model and platform-specific guarantees.
