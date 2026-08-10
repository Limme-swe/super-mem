# Changelog

Notable changes are recorded here. Versioning follows Semantic Versioning. Before 1.0, minor releases may contain breaking changes.

## [Unreleased]

### Added

- Fail-closed, non-creating `supermem doctor` diagnostics for a versioned schema
  manifest, bounded SQLite/FK and canonical-relationship integrity,
  writer-lock availability, database and sidecar alias safety, binary identity,
  redacted scope environment, and deadline/output-bounded Git probes. A pinned
  source identity, descriptor-owned SQLite lock protocol, and immutable private
  snapshot keep live WAL and rollback-journal evidence from being opened,
  checkpointed, recovered, chmodded, or migrated by diagnostics. SQLite
  inspection has a shared five-second work deadline and value-size limits;
  Windows holds its capped snapshot in memory, uses 128-bit local file IDs, and
  does not inherit temporary-directory permissions.
- Deterministic, bounded code aliases for compound identifiers, paths, symbols, and a conservative coding/error concept lexicon.
- Optional immutable search profiles for background document expansions and caller-generated dense vectors, without model calls or downloads in memory writes or recall.
- `supermem index add-profile`, `list-profiles`, `activate`, `deactivate`, `remove-profile`, `pending`, `register`, `status`, and `rebuild` operator commands, plus optional dense-query inputs for CLI and MCP recall.
- A labeled retrieval starter fixture and qrels covering paraphrases, exact symbols and errors, scope isolation, Git applicability, and revision correction. The fixture is a regression seed, not a published quality benchmark.

### Changed

- Recall now separates strict all-term and loose any-term FTS channels for canonical text and code aliases, adds a semantic-expansion channel, and fuses them with existing exact, diagnostic, artifact, entity, sparse, and recency signals. Expansion profiles use independent FTS rows so one profile cannot truncate another profile's text.
- Recall stages bounded body previews and precomputed fixed-width artifact applicability fingerprints, round-robins at most 128 distinct real paths across ranked candidates, bounds the MMR pool, and hydrates full provenance only for pinned selected revisions. Candidate materialization no longer scales with the combined size of up to 1,024 full memory bodies or multi-KiB artifact metadata, one artifact-heavy candidate cannot deny filesystem verification to the rest, and missing or corrupt derived fingerprints cannot prove exact applicability.
- `supermem index artifact-status` recomputes scoped artifact projection integrity, reports missing, corrupt, or orphaned rows without exposing artifact metadata, and distinguishes rebuildable projection damage from missing canonical artifact data.
- Dense retrieval uses exact cosine scoring for up to 4,096 eligible projections. Larger sets stream deterministic 128-bit random-hyperplane signatures into a 512-item Hamming shortlist followed by exact cosine reranking.
- Automatic session checkpoints retain every captured command as event evidence but promote only verification results, failed attempts, and explicitly salient results into standalone outcome memories. Repeated verification runs with a stable command identity coalesce under a revision key and preserve fail-to-pass endpoints; generic tool names never merge unrelated runs. Idempotent retries reconstruct canonical identity, ordering, and lifecycle eligibility at the original checkpoint boundary instead of consulting mutable heads.
- Database schema v6 adds search-profile activation, independent per-projection expansion FTS rows, a rebuildable fixed-width artifact fingerprint projection, and a reverse link-revision index. JSONL snapshots continue to contain canonical state only and omit derived projections.

### Security

- Background projection registration is exact-scope and atomic, passes expansion text through the configured redaction pipeline, validates dense-vector shape and values, rejects stale work by comparing the current revision and content hash, and rejects nondeterministic bytes for an unchanged profile input.

## [0.1.0] - 2026-08-06

### Added

- Local SQLite/FTS5 memory engine, context compiler, CLI, MCP server, and harness adapters.
- Lossless snapshot export and atomic empty-store import.
- Tool, command, test, and file-result capture for the Codex, Claude Code, OpenCode, and Pi adapters.
- Explicit artifact fingerprinting for record and recall operations, plus all-or-nothing changed-file capture for checkpoints.
- CLI and MCP history inspection with immutable events, per-revision metadata fidelity, and revision-scoped links.
- Executable fixture-contract validation for the checked-in coding-memory scenarios.
- Prebuilt release archives for Linux x86-64, Windows x86-64, and both Apple Silicon and Intel macOS, containing the native executable, adapters, documentation, and tracked source tree, with checksums and build-provenance attestations.

### Changed

- Batch candidate, feedback, evidence, entity, artifact, and tag hydration while preserving provenance and deterministic order.
- Contentless, rebuildable FTS5 projection and selective entity/artifact indexes through atomic schema migration.
- Incremental diversity selection with the same IDs, scores, and order; scope, lifecycle, kind, and time eligibility now run before channel limits.
- Workspace-aware canonical identity, idempotency, revisions, evidence, links, and attachment metadata.
- Database schema v4 adds a session-scope index, immutable per-revision metadata, and revision-scoped link history. Migrated pre-v4 non-head revisions retain exact text and mark reconstructed metadata as incomplete.
- Snapshot schema v2 preserves revision metadata and link history while continuing to import v1 snapshots. Snapshot round trips preserve canonical rows, floating-point bits, integrity footers, and tied result order.
- Divergent history is excluded by default and available through an explicit recall option. Complete matching artifact sets can establish exact applicability despite unrelated dirty changes; partial automatic Git capture is discarded.
- Structured and rendered context use the same selected, budgeted bodies and final-render token estimate.

### Performance

- Lower CLI and hook startup cost, blocking MCP work moved off the transport thread, a bounded 2–4 connection MCP recall pool for concurrent agents, and bounded-memory Unicode handling in the OpenCode and Pi adapters.

### Security

- Bounded, domain-separated, length-framed automatic idempotency keys.
- Native-byte and trailing-newline repository identity handling; scope-sensitive commands reject repository-local database paths that could alter tracked Git state through non-ignored files, links, or aliases.
- Full-store purge removes the rollback journal as well as the database, WAL, and shared-memory sidecars after rejecting database or sidecar symlinks, user-controlled symlink or Windows reparse-point components, and hard-link aliases. The fixed macOS `/var` and `/tmp` system aliases are accepted.

[Unreleased]: https://github.com/Limme-swe/super-mem/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Limme-swe/super-mem/releases/tag/v0.1.0
