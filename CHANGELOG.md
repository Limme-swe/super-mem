# Changelog

Notable changes are recorded here. Versioning follows Semantic Versioning. Before 1.0, minor releases may contain breaking changes.

## [Unreleased]

### Added

- Deterministic, bounded code aliases for compound identifiers, paths, symbols, and a conservative coding/error concept lexicon.
- Optional immutable search profiles for background document expansions and caller-generated dense vectors, without model calls or downloads in memory writes or recall.
- `supermem index add-profile`, `pending`, `register`, `status`, and `rebuild` operator commands, plus optional dense-query inputs for CLI and MCP recall.
- A labeled retrieval starter fixture and qrels covering paraphrases, exact symbols and errors, scope isolation, Git applicability, and revision correction. The fixture is a regression seed, not a published quality benchmark.

### Changed

- Recall now separates strict all-term and loose any-term FTS channels for canonical text and code aliases, adds a semantic-expansion channel, and fuses them with existing exact, diagnostic, artifact, entity, sparse, and recency signals.
- Dense retrieval uses exact cosine scoring for up to 4,096 eligible projections. Larger sets stream deterministic 128-bit random-hyperplane signatures into a 512-item Hamming shortlist followed by exact cosine reranking.
- Database schema v5 adds rebuildable alias-version state, immutable generator profiles, current-revision search projections, and an indexed error-fingerprint lookup. JSONL snapshots continue to contain canonical state only and omit search profiles and projections.

### Security

- Background projection registration is exact-scope and atomic, passes expansion text through the configured redaction pipeline, validates dense-vector shape and values, and rejects stale work by comparing the current revision and content hash.

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
