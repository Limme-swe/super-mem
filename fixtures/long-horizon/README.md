# Long-horizon coding-memory benchmark

This fixture is the repository's deterministic production-engine benchmark for memory that must remain useful across sessions, revisions, failures, repository boundaries, workspace boundaries, artifact changes, and diagnostic recurrence.

It contains **32 cases and more than 300 writes** across eight scenario families:

- decisions recalled in later sessions among low-value distractors;
- verified procedures ranked above failed attempts;
- current revisions replacing retired claims;
- repository isolation;
- workspace isolation;
- artifact-freshness exclusion;
- exact diagnostic fingerprints;
- code-identifier aliases.

The fixture is generated rather than hand-maintained:

```sh
python scripts/generate_long_horizon_fixture.py --check
node scripts/validate-retrieval-fixture.mjs fixtures/long-horizon
```

Run it through the production `MemoryEngine` and write a machine-readable report:

```sh
python scripts/run_long_horizon_benchmark.py \
  --output long-horizon-report.json
```

The report includes every case's ordered IDs, scores, relevance grade, retrieval signals, applicability, revision, context-token estimate, rendered size, and warnings. The Rust test hard-fails forbidden-memory exposure, wrong revision use, stale-memory exposure, missing required signals, and explicit rank requirements before a report can be produced.

The benchmark is deterministic and useful for regression and ranking work. It is not a claim about performance on unrelated public conversational-memory benchmarks or stochastic coding agents. Those require separate immutable corpora, equal model budgets, repeated trials, and published raw results.
