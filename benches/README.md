# Benchmark notes

Rust probes live beside measured functions. Normal tests ignore the probes but run their invariant and equivalence checks.

## Available probes

Run the in-process core pipeline probe with:

```sh
cargo test --release -p super-mem-core \
  engine::tests::performance_probe -- --ignored --exact --nocapture --test-threads=1
```

Run the optimized selector against its retained quadratic reference with:

```sh
cargo test --release -p super-mem-core \
  ranking::tests::incremental_mmr_performance_probe \
  -- --ignored --exact --nocapture --test-threads=1
```

Equivalence tests compare every selected ID and score bit on tied and pseudo-random inputs. Engine tests cover recall and attachment order, scope exclusion, and snapshot parity. All must pass after timing changes.

## Planned benchmark coverage

The probes do not cover every production path. A complete suite should include:

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

## Corpus design

Use deterministic fixtures at 1k, 10k, and 100k events. Add a 1M-event run when practical.

Synthetic data should vary:

- Event payload size.
- Repository and branch count.
- Repeated error signatures.
- Supersession chains.
- Tool successes and failures.
- Sparse versus dense symbol links.

Identical tiny records are not a valid corpus.

## Reporting

Retain machine-readable results. Record:

- Git commit and dirty status.
- Hardware, OS, filesystem, and storage medium.
- Rust version and build profile.
- Corpus generator seed and digest.
- Cold/warm cache condition.
- Concurrency and background enrichment settings.
- Median and tail latency.
- Resident memory and database/index size where relevant.

Separate local storage from embedding or remote-model latency.

See [Evaluation](../docs/evaluation.md) for quality methods. Microbenchmarks do not measure memory quality.
