# Architecture

This document defines the intended architecture of `super-mem`. It distinguishes durable evidence from derived memory and makes repository applicability part of retrieval.

## System boundaries

The system has four layers:

1. **Capture adapters** translate harness lifecycle events into a shared event envelope.
2. **Core memory** validates, redacts, stores, and derives memory records.
3. **Retrieval and context assembly** filter and rank evidence for a query and budget.
4. **Interfaces** expose the core through the `supermem` CLI and MCP.

Harness adapters should remain thin. Memory semantics, scoping, conflict handling, and ranking belong in the Rust core so behavior does not drift between clients.

## Source of truth

The authoritative layer is an append-only sequence of observable events. A durable event includes:

| Field | Meaning |
| --- | --- |
| `event_id` | Stable content or event identifier. |
| `recorded_at` | When `super-mem` accepted the event. |
| `observed_at` | When the represented action or statement occurred, if known. |
| `actor` | User, agent, tool, hook, or importer. |
| `session` / `turn` | Explicit harness identifiers; never inferred from a transport connection. |
| `scope` | Identity, organization, workspace, repository, branch, path, and symbol constraints. |
| `repo_state` | Repository identity, HEAD/tree, branch, and dirty-patch digest. |
| `kind` | Prompt, tool call, tool result, patch, validation, correction, decision, or annotation. |
| `payload` | Redacted observable content or a reference to it. |
| `provenance` | Original harness, file, Git object, tool call, or import reference. |
| `integrity` | Digest of normalized content and relevant metadata. |

Search indexes and active-state tables are projections. A damaged or incompatible projection should be rebuildable from accepted events.

## Derived record types

### Claims

A claim is a queryable proposition derived from one or more events. It carries:

- Kind: fact, preference, constraint, state, plan, or relationship.
- Confidence and authority class.
- Supporting and contradicting evidence IDs.
- Valid-time and recorded-time intervals.
- Scope and Git applicability.
- Lifecycle state: active, contested, superseded, or retracted. Git applicability is classified separately.

An extractor may propose a claim; it cannot erase source evidence.

### Decisions

A decision records the selected option, explicit rationale, constraints, alternatives when available, affected repository entities, and subsequent outcome evidence. Only user-visible rationale or an explicit annotation is retained.

### Episodes

An episode represents an attempted task:

- Task description and starting repository state.
- Observable tools and commands.
- Patches and affected symbols.
- Diagnostics and failures.
- Validation evidence.
- Final status and explicit feedback.

An episode is not automatically a reusable procedure.

### Procedures and known failures

A procedure is a parameterized action pattern with preconditions, expected observations, postconditions, and supporting episodes. A known failure records an action pattern, the conditions under which it failed, and the observed failure signature.

Promotion should be conservative. A single apparently successful episode remains an episode unless a user approves it or compatible evidence repeats it.

## Repository identity and applicability

Chronological recency is insufficient for source code. Applicability is evaluated against Git state.

An implementation should capture:

- A repository identity independent of the local checkout path.
- HEAD commit and tree OIDs.
- Current branch when meaningful.
- A digest of tracked and relevant untracked changes.
- Path and stable symbol references where available.

For a memory `m` and current repository state `r`, retrieval classifies applicability:

| Class | Meaning |
| --- | --- |
| `exact` | Same repository state and relevant patch state. |
| `compatible` | The source commit is in the current lineage and linked artifacts remain compatible. |
| `stale` | A linked file or symbol changed enough that the conclusion requires revalidation. |
| `divergent` | The memory belongs to another branch lineage. It may be useful as history, not current truth. |
| `unversioned` | The memory is intentionally repository-independent or lacks version evidence. |
| `inapplicable` | The namespace or repository differs. This is a hard exclusion before ranking. |

Two branch-specific claims may both be valid. Divergence is not itself contradiction.

Paths are weak identities. When language support is available, a symbol reference should combine language, qualified name, signature digest, syntax digest, file path, and source commit. Rename and diff evidence may remap it; uncertain remapping must lower confidence rather than silently attach a memory to the wrong symbol.

## Conflict resolution

Conflict handling is domain-specific:

- For a user preference, an explicit user correction has the highest authority.
- For a code-state claim, an observation from the current checkout or validation at the matching commit outranks an inference.
- For a project decision, checked-in documentation or an explicitly accepted decision may outrank an agent summary.

The resolver may create a `supersedes` edge when authority, scope, and chronology make the relationship unambiguous. Otherwise it marks a contested set. The context assembler should surface an unresolved conflict rather than selecting a convenient winner.

Retraction and supersession differ. Supersession preserves a newer current revision; retraction removes a memory from ordinary retrieval while retaining its audit history. On Unix, the current `purge --yes` CLI operation deletes the entire local database and its SQLite sidecars after refusing symbolic links and paths with multiple hard links. V0.1 refuses purge on Windows because stable Rust does not expose the hard-link count and the workspace forbids unsafe platform FFI. All processes using the database must be stopped first because SQLite cannot portably detect idle open handles. Item-level erasure is not part of the initial interface.

## Capture pipeline

```text
harness event
    -> envelope validation
    -> scope resolution
    -> secret/path redaction
    -> durable event append
    -> deterministic extraction
    -> active-view update
    -> lexical index update
    -> optional background enrichment
```

The hook path must stay bounded. Expensive embedding, model-assisted extraction, and consolidation belong off the synchronous path. Backpressure must be explicit; silently dropping capture events produces false confidence in memory completeness.

Large tool output should be truncated using a documented head/tail or diagnostic-preserving policy. Store a digest and artifact reference so truncation is visible.

## Query pipeline

### 1. Establish scope

Resolve the caller identity, repository, branch/worktree state, session, and requested historical/current view. Access-control and repository filters happen before candidate ranking.

### 2. Generate candidates

The initial implementation can combine:

- Exact IDs, identifiers, and normalized error signatures.
- Lexical full-text retrieval.
- Structured lookups for active claims, linked symbols, decisions, outcomes, and time.

Semantic embeddings and graph expansion can be added as independent candidate channels. The lexical/exact path must remain functional without an embedding model or network access.

### 3. Rank applicability and utility

Ranking features may include:

- Exact diagnostic or symbol match.
- Lexical relevance.
- Repository and Git applicability.
- Temporal validity.
- Evidence authority and confidence.
- Episode outcome.
- Procedure precondition match.
- Redundancy and contradiction penalties.

Weights are configuration backed by evaluation; they are not product claims.

### 4. Assemble context

Selection is constrained by an explicit budget and should maximize marginal evidence coverage rather than append every top result. The current structured result groups records into stable sections:

1. `constraints_and_preferences`.
2. `decisions`.
3. `attempts_and_outcomes`.
4. `procedures`.
5. `open_tasks`.
6. `relevant_history`.

Stale, divergent, and contested records also produce explicit warnings. Structured items retain applicability, reason codes, token estimates, and evidence citations.

Derived entries should include at least one primary evidence reference. When confidence is low, prefer a compact source excerpt over an unsupported summary.

## Persistence and consistency

The initial workspace uses an embedded SQLite store. The implementation should use transactions for accepted events and active-view changes, and make secondary indexes versioned and repairable.

Important invariants:

- An acknowledged event is durable or the caller receives an error.
- A derived record never points to nonexistent evidence.
- A projection records which event generation it covers.
- Reprocessing the same stable harness event is idempotent.
- Concurrent subagents cannot escape their resolved scope.
- Schema migration is explicit and reversible through export or backup.

The database is not the security boundary. File permissions, identity resolution, encryption choices, host process permissions, and retrieval filtering all matter.

## MCP shape

Use the official [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk). The trusted stdio launch pins a canonical root, namespace, and optional workspace. The server rebuilds current Git scope from that root on every call. The 2026-07-28 MCP revision is stateless and removes protocol-level sessions; the model may supply only an optional session provenance value, never a hard isolation boundary.

The public model-facing surface should remain small:

- `memory_context`: scoped recall under an explicit token budget.
- `memory_feedback`: retrieval-quality feedback tied to a memory and optional query.
- `memory_manage`: `inspect` or `retract`; status and database purge remain CLI-only.
- `memory_record`: `record`, `checkpoint`, or `observation` mode.

The initial MCP surface returns compact text content blocks; record and management operations serialize their receipts as JSON text. The core `ContextPack` retains structured hits, applicability, reason codes, and citations for CLI JSON output and future structured MCP content. Tool order and schemas are deterministic to help client prompt caching.
Tool schemas intentionally omit namespace, working directory, repository, and
workspace fields, and reject unknown fields so a caller cannot spoof them.

## Deliberate limitations

- Event order establishes `followed_by`, not causation.
- A green test is positive evidence, not proof that a change is correct.
- Repository-independent memories require explicit scope.
- Deep symbol identity is language-dependent and must fall back safely.
- Automatic extraction can be wrong; inspection and correction are core operations.

## Related evidence

- [SWE-ContextBench](https://arxiv.org/abs/2602.08316): selected prior coding context can help; incorrect context can hurt.
- [LongMemEval-V2](https://arxiv.org/abs/2605.12493): agent memory must retain state, workflows, environment gotchas, and premise awareness from trajectories.
- [MemoryAgentBench](https://arxiv.org/abs/2507.05257): evaluates retrieval, test-time learning, long-range understanding, and selective forgetting.
- [Memora](https://arxiv.org/abs/2604.20006): exposes obsolete-memory failures under repeated mutation.
- [MemMachine](https://arxiv.org/abs/2604.04853): motivates preserving raw episodic ground truth beneath optimized retrieval.
