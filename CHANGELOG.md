# Changelog

Notable changes are recorded here. Versioning follows Semantic Versioning. Before 1.0, minor releases may contain breaking changes.

## [Unreleased]

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
- Full-store purge removes the rollback journal as well as the database, WAL, and shared-memory sidecars after rejecting symbolic links, Windows reparse points, and hard-link aliases.

[Unreleased]: https://github.com/Limme-swe/super-mem/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Limme-swe/super-mem/releases/tag/v0.1.0
