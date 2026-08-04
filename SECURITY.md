# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's **Report a vulnerability** security-advisory
flow. Do not open a public issue for secret leakage, scope-isolation failures, prompt-injection paths,
or unsafe deletion behavior.

## Security model

Super-mem is local-first and has no telemetry. Its database may contain prompts, command results,
repository metadata, and other sensitive context. Protect it as you would protect source code and
terminal history.

The default design:

- redacts common secret shapes before persistence;
- scopes recall before ranking to reduce cross-project leakage;
- marks retrieved memories as untrusted evidence;
- does not expose arbitrary file reads through MCP;
- keeps permanent purge as an explicit CLI-only operation.

Redaction is defense in depth, not a guarantee. Do not intentionally store credentials. SQLite WAL
files, backups, filesystem snapshots, and SSD behavior also mean forensic erasure cannot be guaranteed.
