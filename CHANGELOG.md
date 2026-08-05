# Changelog

All notable changes will be documented here. The project follows Semantic Versioning after the first
stable release.

## [Unreleased]

- Initial evidence-first, Git-aware memory engine.
- SQLite/FTS5 local store, context compiler, CLI, MCP server, and harness adapters.
- Batch candidate hydration, feedback, evidence, entity, artifact, and tag reads while preserving
  complete provenance and deterministic ordering.
- Replace duplicate FTS payload storage with a rebuildable contentless FTS5 projection and add
  selective entity/artifact indexes through an atomic schema migration.
- Make broad diversity selection incremental without changing its selected IDs, scores, or order;
  apply lifecycle, temporal, kind, and scope eligibility before per-channel limits.
- Preserve snapshot floating-point values bit-for-bit, keep the canonical snapshot format compatible
  across the derived-index migration, and total-order tied retrieval results.
- Make workspace identity part of canonical identity, idempotency, explicit revision, evidence, and
  memory-link isolation; partition attachment metadata by durable scope so another workspace cannot
  overwrite it. Schema v3 adds the supporting lookup indexes while the snapshot schema remains v1.
- Reduce one-shot CLI and hook startup overhead, move blocking MCP operations off the async
  transport thread, and avoid whole-message Unicode copies in the OpenCode and Pi adapters.
- Derive automatic capture idempotency keys from bounded, domain-separated, length-framed tuples so
  long host identifiers and delimiter characters cannot collide or exceed the core key limit.
- Harden repository identity for native-byte and newline-ending paths, and reject scope-sensitive
  use of repository-local databases that could mutate tracked Git state through non-ignored files,
  links, or aliases.
