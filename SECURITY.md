# Security policy

## Supported versions

Until a stable release policy exists, security fixes target current `main` and the newest tagged 0.x
release. Older releases and arbitrary commit snapshots are not maintained.

| Version | Security updates |
| --- | --- |
| Current `main` / newest tagged 0.x | Yes |
| Older 0.x releases and snapshots | No |

## Reporting a vulnerability

Report vulnerabilities through GitHub's private
[Report a vulnerability](https://github.com/Limme-swe/super-mem/security/advisories/new) flow. Do not
open a public issue for secret leakage, scope-isolation failures, prompt-injection paths, unsafe
deletion behavior, or another exploitable condition.

Include the affected version or commit, impact, minimal reproduction, and any known mitigation. Use
synthetic data: do not include real credentials, private source, repository contents, or memory
databases. Maintainers will review reports as capacity allows and coordinate disclosure after a fix or
mitigation is available. No response-time guarantee is offered.

## Security model

super-mem is local-first and has no telemetry. Its database may contain prompts, command results,
repository metadata, and other sensitive context. Protect it as you would protect source code and
terminal history.

The default design:

- redacts common secret shapes before persistence in the default configuration;
- scopes recall before ranking to reduce cross-project leakage;
- marks retrieved memories as untrusted evidence;
- does not expose arbitrary file reads through MCP;
- keeps permanent purge as an explicit CLI-only operation.

Redaction is defense in depth, not a guarantee. Do not intentionally store credentials. SQLite WAL
files, backups, filesystem snapshots, and SSD behavior also mean forensic erasure cannot be guaranteed.
