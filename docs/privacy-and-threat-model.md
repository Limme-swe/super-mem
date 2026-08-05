# Privacy and threat model

Automatic agent memory observes unusually sensitive material: user prompts, local paths, source code, shell commands, diagnostics, patches, and tool output. Running locally reduces exposure to a hosted service, but does not make capture safe by itself.

This document defines security requirements for the project. It is not a certification or a claim that every control is complete in an early build.

## Assets

- User and organization preferences or instructions.
- Private repository source and history.
- Tool arguments and results.
- Credentials, tokens, keys, and connection strings accidentally exposed to tools.
- Architectural decisions and security findings.
- Cross-session behavioral history.
- Memory database, indexes, exports, backups, and logs.

## Trust boundaries

### Local user boundary

The local operating-system account is the default administrative boundary. The initial store is plaintext SQLite: a process with permission to read the database can read its contents. Use operating-system file permissions and disk/filesystem encryption where needed; application-level encryption is not implemented yet.

### Harness boundary

Codex, Claude Code, OpenCode, Pi, and generic MCP clients differ in what lifecycle data they expose and which identity they attach. An adapter must not infer identity or repository scope solely from a current working directory supplied by an untrusted event. The local MCP server instead pins a canonical root, namespace, and optional workspace in its trusted launch configuration and rediscovers Git state from that root on every call.

### Repository boundary

A checked-out repository is untrusted input. It may contain malicious instructions, symlinks, generated files, adversarial text, or paths intended to confuse scope resolution.

### Model boundary

A language model can request tools and produce annotations, but its output is not authoritative. It must not bypass access control, silently promote a claim to policy, or physically delete evidence without the same authorization required of a human-facing client.

## Principal risks

### Secret capture

A prompt, `.env` file, compiler output, stack trace, URL, or shell command may contain a secret. Redaction after indexing is too late because the secret may persist in the event log, full-text index, backups, or temporary files.

Required controls:

- Apply path exclusions and content redaction before durable append.
- Recognize common credential forms without logging the matched value.
- Exclude environment dumps and known credential directories by default.
- Bound captured tool output and avoid storing entire files through generic read tools.
- Make capture failure visible when safe redaction cannot be guaranteed.
- Apply the same policy to exports and diagnostics.

No regex-based detector catches every secret. Users should still avoid placing credentials in prompts and should rotate any credential known to have been captured.

### Cross-scope leakage

Semantically similar material from another repository or user can be highly ranked. Filtering those results after retrieval risks side channels and implementation mistakes.

Required controls:

- Resolve caller identity and allowed scopes before candidate generation.
- Never accept namespace, root, repository, or workspace boundaries from model tool arguments.
- Include repository identity in exact, lexical, structured, and future vector indexes.
- Default to repository isolation; cross-repository memory is opt-in.
- Keep global preferences separate from repository facts.
- Add negative tests for duplicate text across denied scopes.

### Prompt injection through memory

Captured source, issues, logs, and previous assistant text may contain instructions such as “ignore previous rules.” Recalling that text into a developer or system context can elevate untrusted data.

Required controls:

- Label recalled content as evidence, not instruction.
- Keep provenance and content boundaries machine-readable.
- Never promote an instruction from arbitrary tool output.
- Only explicit, authorized memory operations may create a procedural rule or preference.
- Prefer structured fields over interpolating raw evidence into control text.
- Preserve repository policy in the harness's instruction mechanism, not memory.

### Memory poisoning

An agent may store a plausible but false claim, a failed workaround, or an attacker-controlled statement. Repetition can then make the false item appear authoritative.

Required controls:

- Separate assertions, inferences, observations, and validations.
- Keep failed attempts outcome-labeled.
- Require provenance for derived records.
- Make promotion thresholds conservative.
- Do not increase authority merely because an item was retrieved repeatedly.
- Support contest, correction, retraction, and audit views.

### Stale or branch-incompatible memory

A formerly correct fact may become unsafe after code changes or may apply only on another branch.

Required controls:

- Bind repository claims to Git and artifact state.
- Mark changed or divergent evidence instead of returning it as current.
- Surface stale evidence when it is useful historically.
- Require revalidation before stale procedures are promoted again.

### Local database compromise

Malware or another process running as the user may read or alter local files.

Required controls:

- Restrictive file and directory permissions.
- Checksums or integrity metadata for event records.
- Crash-safe transactions and recoverable backups.
- No secrets in process arguments where they may appear in process listings.
- If application-level encryption is added, it must use an OS-protected key or explicit passphrase and authenticated encryption.

Encryption at rest does not protect memory while the service is running under a compromised account.

The reference adapters use the CLI's `--*-stdin` inputs for captured content, so prompts and responses are not placed in process arguments. Integrators should preserve that property: the convenience flags that accept literal text are suitable for interactive, non-secret input, while automatic capture should use stdin or a future authenticated local-IPC envelope.

Standard input is not an authentication boundary. On a hostile or compromised host, process inspection and same-user tracing may still expose data; operating-system isolation remains part of the trust model.

### Malicious paths and symlinks

Repository-controlled symlinks can redirect an adapter toward files outside the repository.

Required controls:

- Canonicalize roots and validate containment without trusting lexical `..` removal alone.
- Avoid following repository symlinks for capture by default.
- Treat nested repositories and worktrees as separate resolved identities.
- Never recursively delete a path derived only from an event.

The memory database can itself affect repository truth when placed inside a
worktree. Before a scope-sensitive command, hook, or MCP operation opens
SQLite, the Unix CLI therefore requires all four possible paths (the main file
plus `-wal`, `-shm`, and `-journal`) to be untracked and Git-ignored, and
rejects symbolic links, symlinked ancestors, multiple hard links, tracked
aliases, and `..` components. Non-Unix v0.1 builds reject repository-local
database paths on these operations. An external canonical database path is the
portable default. The CLI's non-scoped `init`, `inspect`, `feedback`, `retract`,
`status`, `doctor`, `export`, `import`, and `purge` commands intentionally do
not apply this Git-applicability guard.

### Denial of service

Large outputs, repeated hooks, malformed MCP payloads, and many parallel subagents can exhaust disk, memory, or CPU.

Required controls:

- Request and record-size limits.
- UTF-8-safe automatic lifecycle capture capped at 64 KiB with an explicit truncation marker.
- Bounded queues and concurrency.
- Stable-event deduplication.
- Timeouts and cancellation.
- Storage quotas and inspectable retention policy.
- Explicit degraded behavior rather than silent data loss.

## Data minimization

Default capture should retain the minimum evidence needed to reconstruct an outcome:

- Tool name, normalized arguments, status, diagnostic excerpt, and digest.
- Patch metadata and affected artifacts rather than duplicate full repository files.
- Visible assistant output only where it contributes evidence.
- No hidden chain-of-thought or private model state.
- No telemetry unless separately implemented and explicitly enabled.

Model-assisted extraction and remote embeddings must be opt-in and disclose exactly what content leaves the machine.

## Retraction and purge

The controls have different semantics:

- **Supersede:** retain history and choose a newer current claim.
- **Retract:** state that a claim should no longer be considered valid.
- **Purge:** permanently delete the entire local database plus its SQLite WAL and shared-memory sidecars.

Purge does not delete exports, user-managed backups, filesystem snapshots, or copies held by a harness. On flash storage, application-level deletion cannot guarantee physical erasure from every device cell. Documentation and CLI output must not imply otherwise.

Stop every CLI, MCP server, hook, and harness process using the store before
purging it. SQLite cannot portably identify an idle process that merely retains
an open database handle; such a process may retain or recreate WAL state after
another process unlinks the paths. Purge therefore rejects symbolic links and,
on Unix, multiply hard-linked database or sidecar paths, but it cannot revoke
open handles or delete external copies.

The v0.1 CLI refuses purge on Windows. Rust's stable Windows metadata API does
not expose the hard-link count, and this workspace forbids unsafe platform FFI;
reporting success without verifying that no linked name survives would violate
the deletion contract. Windows users must stop all processes and use audited OS
administration tooling to remove the database, sidecars, and any linked names.

Physical purge is a human-facing CLI operation with explicit confirmation. It is not exposed as a model-callable MCP operation.

## MCP considerations

The [MCP tools specification](https://modelcontextprotocol.io/specification/2026-07-28/server/tools) describes tools as model-controlled and recommends that applications preserve meaningful user control. Memory mutation deserves particular visibility; destructive purge remains outside the MCP surface.

The 2026-07-28 protocol is stateless. For local stdio, trusted launch arguments pin the root, namespace, and optional workspace; model calls cannot override them. An optional model-supplied session ID is provenance only. Remote Streamable HTTP support, if added, must authenticate every request and enforce the same pre-retrieval filters as local stdio.

## Out of scope

This design does not protect against:

- A fully compromised user account or kernel.
- A malicious harness with authorized direct access to the memory database.
- Secrets that evade detection and are legitimately included in allowed captured content.
- Copies made outside `super-mem` exports and managed backups.
- Incorrect conclusions made by a model after receiving accurate evidence.

## Reporting

Security reports should include the affected version/commit, deployment mode, minimal reproduction, expected scope, and whether sensitive data was exposed. Do not include live credentials or private repository contents in a public issue.
