# Security and privacy

Agent memory can contain prompts, source paths, commands, diagnostics, patches, and tool output. super-mem keeps this data local, but local storage is not automatically safe.

The database is plaintext SQLite. Protect it as you would protect source code and terminal history.

## Data handled by super-mem

A store may contain:

- user prompts and visible assistant output;
- commands, normalized tool arguments, results, and error messages;
- repository identity, branches, commits, paths, and dirty-state digests;
- decisions, procedures, failed attempts, and task checkpoints;
- evidence links, feedback, exports, and backups.

The reference adapters do not request or capture hidden model reasoning. super-mem has no telemetry.

## Trust boundaries

| Boundary | Assumption |
| --- | --- |
| Operating-system account | Processes running as the same user may be able to read or alter the database. |
| Harness | The harness controls which lifecycle events are exposed and when hooks run. |
| Repository | Checked-out files, paths, symlinks, and text are untrusted input. |
| Model | Model output can request tools but is not an authority boundary. |
| MCP launch | Trusted launch arguments pin root, namespace, and optional workspace. |

Standard input keeps captured text out of process arguments, but it is not authentication. A hostile process running as the same user may still inspect memory or process state.

## Current controls

### Secret handling

With the default engine configuration, accepted text and metadata are checked for common secret patterns before they are written. Sensitive scalar values are replaced with constant redaction markers, and snapshot import applies the same validation.

Automatic lifecycle capture is limited to 64 KiB with a visible head/tail truncation marker. The reference adapters send captured text through stdin rather than command arguments.

These checks reduce accidental exposure; they do not detect every credential. Do not intentionally store secrets. If a real credential reaches the store, rotate it.

### Scope isolation

Recall filters namespace, workspace, repository, lifecycle state, and time before channel limits and ranking. Repository identities are derived from Git metadata and native path bytes rather than display strings.

The MCP server pins its root, namespace, and optional workspace at launch. Model tool arguments cannot replace those values. Cross-repository or cross-workspace evidence is rejected during writes, links, revisions, and recall.

### Untrusted recalled text

Adapters label recalled content as untrusted evidence inside a marked envelope. Some hosts, including OpenCode, provide a system-prompt field as the injection point; the label does not guarantee how a model will interpret the content.

Repository policy belongs in AGENTS.md, CLAUDE.md, checked-in documentation, or another trusted harness instruction source. A context tag helps clients preserve the boundary but does not make malicious text safe.

### Poisoning and stale knowledge

Records keep source evidence, outcome, revision history, revision-scoped links, and feedback. Failed attempts remain distinct from successful procedures. Corrections can supersede or contest older records without deleting their history. CLI and MCP history inspection exposes this audit data to an authorized caller.

Repository memories are checked against current Git and artifact state. Stale records require an explicit stale-data option, and divergent records require a separate opt-in. A complete matching artifact set can establish exact applicability despite unrelated dirty files; incomplete automatic Git capture is discarded rather than treated as proof.

### Local storage

New database directories and files use restrictive permissions where the platform supports them. The default balanced mode uses SQLite WAL with synchronous=NORMAL: it preserves consistency across ordinary process crashes, but the newest acknowledged commit may be lost after an operating-system or power failure. Snapshot v2 exports include revision metadata, historical links, table counts, and an integrity footer. Import is atomic, only restores into an otherwise empty store, and accepts v1 snapshots. Historical non-head metadata unavailable in a legacy store or snapshot is marked incomplete rather than presented as exact.

The database application ID is checked before destructive operations. Search indexes are derived state and can be rebuilt from canonical rows.

## Database paths

The portable default is a database outside the repository.

For scope-sensitive commands, hooks, and MCP, Unix builds allow a repository-local database only when:

- the main file and possible -wal, -shm, and -journal sidecars are untracked and Git-ignored;
- no path component is a symbolic link;
- existing files have one hard link;
- the supplied path contains no .. component.

Both logical and resolved paths are checked. These rules keep SQLite writes from changing the Git state used to classify memories.

Non-Unix v0.1 builds reject repository-local database paths for scope-sensitive operations because hard-link aliases cannot be verified safely with the current safe Rust implementation.

The unscoped init, inspect, feedback, retract, status, doctor, export, import, and purge commands do not apply the Git-state guard. Their other path and database-identity checks still apply.

## Retraction and deletion

The operations have different meanings:

| Operation | Result |
| --- | --- |
| Supersede | Keep history and make a newer revision current. |
| Retract | Remove a memory from ordinary recall while retaining its audit history. |
| Purge | Delete the complete local database, WAL, shared-memory, and rollback-journal files. |

Purge is available only through the human-facing CLI and requires explicit confirmation. It is not exposed through MCP.

Stop every CLI, MCP, hook, and harness process using the store before purging. SQLite cannot reliably detect a process that still holds an idle database handle. Such a process may retain or recreate WAL or rollback-journal state after deletion.

Purge refuses symlinks and, on Unix, paths with multiple hard links. V0.1 refuses purge on Windows because safe stable Rust does not expose the hard-link count needed to verify that no linked name survives.

Purge does not remove exported snapshots, user-managed backups, filesystem snapshots, or copies held by another program. Flash storage can also retain physical remnants after application-level deletion.

## Known limitations

- The store is not encrypted by super-mem. Use operating-system permissions and disk encryption where required.
- Pattern-based redaction is incomplete by nature.
- A compromised user account, harness, or kernel can bypass application controls.
- A malicious but authorized harness can store false or sensitive evidence.
- Accurate evidence can still be misunderstood by the consuming model.
- Remote MCP transport and remote embeddings are not implemented.
- Full forensic erasure cannot be guaranteed.

Application-level encryption, if added later, will need authenticated encryption and a key protected outside the database. Encryption alone would not protect a running service under a compromised account.

## Denial of service

Requests, metadata, record content, candidate counts, and automatic capture are bounded. Stable idempotency keys prevent identical hook retries from creating duplicate writes. Hook subprocesses have timeouts and fail open.

A large or hostile local workload can still consume disk, CPU, or memory. Storage quotas and a resident multi-client service are not part of v0.1.

## MCP notes

MCP tools are model-controlled. Memory mutation remains visible through explicit tool calls, while physical purge stays outside the model-facing surface.

The stdio transport inherits the permissions of the process that launches it. The optional session ID is provenance only; it is not an authentication or repository boundary.

## Reporting a vulnerability

Do not open a public issue for secret leakage, scope-isolation failures, prompt-injection elevation, unsafe deletion, or similar vulnerabilities.

Use GitHub's private [security advisory form](https://github.com/Limme-swe/super-mem/security/advisories/new). Include the affected commit, operating system, deployment mode, minimal reproduction, expected scope, and whether data was exposed. Replace real credentials and private source with safe test values.

See [SECURITY.md](../SECURITY.md) for the supported-version policy.
