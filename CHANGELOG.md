# Changelog

Notable changes are recorded here. Semantic Versioning begins with the first stable release.

## [Unreleased]

### Added

- Local SQLite/FTS5 memory engine, context compiler, CLI, MCP server, and harness adapters.
- Lossless snapshot export and atomic empty-store import.

### Changed

- Batch candidate, feedback, evidence, entity, artifact, and tag hydration while preserving provenance and deterministic order.
- Contentless, rebuildable FTS5 projection and selective entity/artifact indexes through atomic schema migration.
- Incremental diversity selection with the same IDs, scores, and order; scope, lifecycle, kind, and time eligibility now run before channel limits.
- Workspace-aware canonical identity, idempotency, revisions, evidence, links, and attachment metadata. Database schema v3 retains snapshot schema v1.
- Snapshot round trips preserve canonical rows, floating-point bits, integrity footers, and tied result order.

### Performance

- Lower CLI and hook startup cost, blocking MCP work moved off the transport thread, and bounded-memory Unicode handling in the OpenCode and Pi adapters.

### Security

- Bounded, domain-separated, length-framed automatic idempotency keys.
- Native-byte and trailing-newline repository identity handling; scope-sensitive commands reject repository-local database paths that could alter tracked Git state through non-ignored files, links, or aliases.
