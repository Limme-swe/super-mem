# Architecture

super-mem stores append-only events, builds searchable records, and filters recall by namespace, workspace, repository, and Git state. Future work is labeled explicitly.

## Components

| Layer | Responsibility |
| --- | --- |
| Capture adapters | Translate harness lifecycle events into a shared envelope. |
| Core memory | Validate, redact, store, and derive records. |
| Retrieval | Filter, rank, and assemble context within a budget. |
| Interfaces | Expose the core through the `supermem` CLI and MCP server. |

Scoping, conflicts, ranking, and persistence stay in the Rust core so adapters behave consistently.

## Canonical data

Canonical state is stored across several SQLite tables. Immutable source observations use the Rust `Event` type:

| Field | Meaning |
| --- | --- |
| `seq` | Database sequence number. |
| `event_id` | Stable event ID. |
| `kind` | `EventKind`: conversation turn, tool call/result, command result, file change, verification, explicit memory, checkpoint, manual note, or lifecycle event. |
| `scope` | Namespace plus optional workspace, repository, and session IDs. |
| `content` | Redacted source text. |
| `attributes` | Structured redacted metadata. |
| `trust` | `External`, `Agent`, `ToolVerified`, or `UserConfirmed`. |
| `occurred_at` | Time reported by the source. |
| `ingested_at` | Time committed by super-mem. |
| `redaction_count` | Number of redacted secret-shaped values. |

When `scope.repository` is present, `RepositoryContext` contains:

| Field | Meaning |
| --- | --- |
| `repo_id` | Stable identity supplied by the adapter. |
| `root` | Optional normalized local root, used as metadata. |
| `common_dir` | Git common directory for worktrees and submodules. |
| `branch` | Current branch. |
| `head_oid` | Current commit ID. |
| `remote` | Normalized remote URL or opaque caller-supplied identity. |
| `dirty_hash` | Fingerprint of tracked and untracked changes. |

Canonical snapshot state includes events, memory heads and revisions, per-revision metadata, evidence, tags, entities, artifacts, current and historical links, event-memory mappings, feedback, and idempotency records. Contentless FTS, deterministic aliases, and optional search profiles and projections are derived state; profiles and projections are omitted from JSONL snapshots and must be registered again after import. Events alone cannot reconstruct the current tables.

## Memory kinds

`MemoryKind` has nine values:

| Kind | Meaning |
| --- | --- |
| `Fact` | Potentially verifiable proposition. |
| `Preference` | User or team preference. |
| `Constraint` | Requirement or invariant. |
| `Decision` | Choice and rationale. |
| `Procedure` | Reusable method. |
| `Episode` | Task or session summary. |
| `Outcome` | Successful, failed, or partial attempt. |
| `Task` | Work that remains. |
| `Observation` | Low-level observation from the event stream. |

Each memory head is `Active`, `Contested`, `Superseded`, or `Retracted` and points to its current revision. Revisions retain their own kind, lifecycle state, canonical key, scores, trust, validity window, event spans, artifacts, entities, and tags. Links retain the source revision that created them. Writing a memory cannot erase its source event. `inspect --history` and MCP `memory_manage` history return this ledger, including feedback and a metadata-fidelity flag for every revision. Reference adapters do not request or capture hidden model reasoning.

## Repository applicability

Built-in Git discovery hashes a normalized non-local `origin` remote when available. For a local or missing remote, it hashes the native bytes of the canonical common Git directory, falling back to the repository root. Linked worktrees share the common-directory identity. `RepositoryContext` also carries root/common directory, branch, HEAD, remote, and dirty-worktree state; artifact references add path, symbol, content hash, Git object ID, and language.

Recall assigns one applicability class:

| Class | Meaning |
| --- | --- |
| `exact` | Same repository state, or a complete set of matching artifact content hashes. |
| `compatible` | Stored commit is an ancestor of the current commit. |
| `stale` | A matching artifact changed, or dirty state differs without complete artifact verification. |
| `divergent` | Stored commit is a descendant, Git history diverged, or branch names differ without commit data. |
| `unversioned` | Repository or Git data is insufficient for classification. |
| `inapplicable` | Namespace, required workspace, or repository scope is incompatible; excluded before ranking. |

Divergent memories can both be valid, but normal recall excludes them; callers must use `--include-divergent` or the corresponding MCP option. Artifact comparison matches repository ID, path, and optional symbol before comparing content hashes. If every stored artifact is re-fingerprinted and matches, unrelated dirty changes do not make the memory stale. Automatic changed-file capture is all-or-nothing: a partial Git result is discarded and cannot establish exact applicability. Language-aware symbol identity and rename remapping are future work.

## Lifecycle links, trust, and removal

Lifecycle changes are explicit. A `supersedes` link marks its target superseded. A `contests` link marks both source and active target contested. A same-kind revision of a contested head returns it to active unless the revision adds another contest. There is no automatic promotion or domain-specific conflict resolver.

Trust is a ranking factor, not proof of truth: `External`, `Agent`, `ToolVerified`, and `UserConfirmed` receive increasing weights. Supersession and retraction preserve history; retracted memories are excluded from normal recall.

On Linux, macOS, and Windows, `purge --yes` removes the database, WAL, shared-memory, and rollback-journal files after rejecting database or sidecar symlinks, user-controlled symlink or Windows reparse-point components, and multiple hard links. The fixed macOS `/var` and `/tmp` system aliases are accepted. Windows obtains the link count from a native file handle without elevation. Stop all database users first; SQLite cannot portably identify idle open handles. Item-level physical erasure is unavailable.

## Capture

```text
harness event
    -> envelope validation
    -> scope resolution
    -> secret redaction
    -> transactional event insert
    -> optional checkpoint or memory revision
    -> lexical index update when memory changes
```

All four reference adapters record exposed tool, command, test, and file outcomes as immutable event evidence. Test, lint, type-check, and build commands are marked as verification evidence. Session checkpoints promote verification results, failures, and explicitly salient results into reusable outcome memories; ordinary successful inspection commands remain evidence instead of becoming standalone outcomes. Repeated verification runs are coalesced, fail-to-pass transitions retain both endpoints, and the stable automatic outcome key revises one head while preserving revision history. Automatic hook capture is capped at 64 KiB. Oversized UTF-8 input keeps its head and tail with an explicit middle-omitted marker. Hook failures fail open so the coding session continues. The Rust hook prints errors to stderr, but the OpenCode and Pi adapters discard that stream, so diagnostics are not always visible.

`remember`, `recall`, and `checkpoint` accept repeatable repository-relative `--file` arguments. Checkpoints also attempt to hash every staged, unstaged, deleted, and untracked path. Automatic capture attaches only a complete bounded set; explicit paths can be used when targeted evidence is preferable.

Optional document expansions and dense vectors are generated by caller-owned workers and registered after the canonical write commits. The core never invokes or downloads a model during writes or recall. Exact, structured, and lexical retrieval continue without generated projections, downloads, or network access; see [Search indexing](search-indexing.md) for the operator workflow.

## Recall

Recall runs in four stages:

1. **Scope:** resolve identity, repository, branch/worktree, session, and current or historical view. Access filters run before ranking.
2. **Candidates:** combine literal title/body matches and exact diagnostics, strict and loose lexical FTS, deterministic code aliases, structured lookups, registered expansions, and an optional caller-supplied dense-vector query. Scope, lifecycle, kind, and time predicates run before channel limits so ineligible rows cannot crowd out eligible ones. Up to 1,024 fused candidates stage scalar metadata, hashed artifact applicability data, and a 1,024-character body preview. MMR evaluates a deterministic pool of at least four times the requested result limit, bounded between 256 and 512 candidates.
3. **Ranking:** fuse exact text, error fingerprint, verified artifact, dense vector, strict and loose lexical, code alias, semantic expansion, sparse identifier/artifact, entity, and recency signals. The score also applies importance, confidence, lifecycle state, age, trust, Git applicability, and feedback utility before diversity selection.
4. **Assembly:** select within budget and emit `constraints_and_preferences`, `decisions`, `attempts_and_outcomes`, `procedures`, `open_tasks`, and `relevant_history`. Warnings cover included stale or divergent records and contested records. Items retain applicability, reasons, token estimates, and citations.

Fusion and diversity selection are totally ordered by score and memory ID. Incremental maximum-redundancy updates are exactly equivalent to full recomputation while reducing broad MMR selection from quadratic-in-output to linear-in-output work per candidate.

Assembly hydrates only the immutable revisions selected by MMR, including their full provenance and attachments. Each selected body is fetched with a bound derived from the complete context budget before exact escaped-fragment budgeting. `ContextPack.sections`, `ContextPack.hits`, and the rendered text therefore use the same selected and safely truncated bodies. The pack's token estimate is computed from the final rendering plus the adapter envelope reserve and does not exceed the requested budget.

Caller-supplied dense vectors and background document expansions are additive channels, not replacements for exact and lexical retrieval. Bundled model execution and graph expansion are not implemented.

## Persistence and invariants

SQLite stores canonical events, revisions, evidence, links, and feedback. Contentless FTS5 is rebuildable; hot statements use a bounded prepared cache, and related rows load in batches.

Schema v2 added contentless FTS and entity/artifact indexes. Schema v3 made canonical lookup workspace-aware and indexed scope-partitioned attachments. Schema v4 adds a session-scope event index, immutable per-revision metadata, and revision-scoped link history. Schema v5 adds deterministic alias-version state, immutable search profiles, current-revision expansion/vector projections, and an error-fingerprint index. Schema v6 adds profile activation, independent per-projection expansion FTS rows, and a reverse link-revision index. Entity display and artifact language metadata remain partitioned by scope, so workspaces cannot overwrite each other's attachments.

Snapshot schema v2 includes the schema-v4 revision metadata and historical links. Import still accepts snapshot v1. When a v1 snapshot or a pre-v4 database lacks historical revision metadata, exact revision text and attachments are retained, but reconstructed non-head metadata is marked incomplete; the current head remains complete. Derived indexes are never snapshot truth.

Artifact and dirty-state checks precede Git history traversal. Commit-DAG queries are lazy and cached per recall by root and stored/current object-ID tuple.

Enforced invariants:

- Derived records cannot point to missing evidence.
- Reprocessing a stable harness event is idempotent.
- Concurrent subagents remain inside their resolved scope.
- Schema migrations are explicit; export and backup provide the recovery path.
- Export, empty-target import, and re-export preserve canonical SQLite scalar values, including exact floating-point bit patterns, revision metadata, link history, and the snapshot integrity footer. Import restores atomically and rebuilds FTS.

SQLite uses WAL. The default `Balanced` mode sets `synchronous=NORMAL`: transactions survive process crashes, but the latest acknowledged commit can be lost after power loss. `Durable` mode sets `synchronous=FULL` and fsyncs every acknowledged commit.

The database is not a security boundary; see the [privacy and threat model](privacy-and-threat-model.md).

The default database is outside the worktree. On Linux, macOS, and Windows, scope-sensitive commands, hooks, and MCP allow a repository-local database only when its main file and possible `-wal`, `-shm`, and `-journal` sidecars are untracked and ignored. Raw and resolved paths reject `..`, symbolic-link or reparse-point components, multiple hard links, and tracked aliases. Existing external database files receive the same alias check so an outside path cannot mutate a tracked hard link. Windows verifies link counts through the native file handle and fails closed if metadata is unavailable. The non-scoped `init`, `inspect`, `feedback`, `retract`, `status`, `doctor`, `export`, and `import` commands skip the Git-applicability guard; purge retains its database-identity and link checks.

## MCP server

The server uses the [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk). Trusted stdio launch pins root, namespace, optional workspace, and the launch repository identity. Every call rediscovers Git state and fails closed if the repository appears, disappears, changes ID, or changes common directory. File-backed servers open one primary connection for writes and a bounded pool of two to four reader connections for concurrent recalls; literal in-memory databases retain one connection so their state stays shared. MCP revision 2026-07-28 is stateless, so a model session ID is provenance, not a security boundary.

The model-facing tools are:

- `memory_context`: scoped recall under a token budget.
- `memory_feedback`: retrieval feedback tied to a memory and optional query.
- `memory_manage`: inspect, load immutable revision, event, link, and feedback history, or retract; status and purge remain CLI-only.
- `memory_record`: record a memory, checkpoint, or observation.

Tools return compact text blocks and JSON-text receipts. `ContextPack` retains the same budgeted bodies in rendered text, sections, and ranked hits, together with applicability, reasons, and citations for CLI JSON. Deterministic schemas reject unknown fields and omit namespace, working directory, repository, and workspace, preventing model calls from spoofing scope.

## Limitations

- Event order establishes `followed_by`, not causation.
- A passing test is evidence, not proof that a change is correct.
- Repository-independent memories require explicit scope.
- Deep symbol identity remains language-dependent and must fall back safely.
- Extraction can be wrong; inspection, feedback, correction, and retraction remain necessary.
