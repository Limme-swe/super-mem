# Benchmark notes

This directory is reserved for reproducible Rust microbenchmarks and their documentation. It intentionally contains no performance numbers yet.

## Required benchmark groups

### Capture

- Append one event.
- Append a tool call/result pair.
- Idempotent duplicate capture.
- Superseding claim projection.
- Batched capture from parallel agents.

### Retrieval

- Exact ID and diagnostic lookup.
- Lexical query.
- Repository-scope filter.
- Git applicability classification.
- Supersession/current-view lookup.
- Context assembly at fixed budgets.

### Maintenance

- Projection rebuild.
- Export/import.
- Retraction and full-store purge.
- Database migration.
- Recovery after an interrupted write.

## Corpus sizes

Run each relevant benchmark at multiple deterministic fixture sizes, initially 1k, 10k, and 100k events. A 1M-event run should be reported only where hardware and runtime make it practical.

Synthetic generation must preserve realistic distributions for:

- Event payload size.
- Repository and branch count.
- Repeated error signatures.
- Supersession chains.
- Tool successes and failures.
- Sparse versus dense symbol links.

Do not use a corpus consisting only of identical tiny records; it produces misleading cache and compression behavior.

## Reporting

Criterion output or another raw machine-readable format should be retained. A summary must include:

- Git commit and dirty status.
- Hardware, OS, filesystem, and storage medium.
- Rust version and build profile.
- Corpus generator seed and digest.
- Cold/warm cache condition.
- Concurrency and background enrichment settings.
- Median and tail latency.
- Resident memory and database/index size where relevant.

Keep local embedded-store timing separate from optional embedding or remote-model latency.

See [Evaluation methodology](../docs/evaluation.md) for quality and end-to-end evaluation. Microbenchmarks establish implementation behavior; they do not establish memory quality.
