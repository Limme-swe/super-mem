# Evaluation

super-mem measures recall, scope safety, freshness, provenance, context cost, latency, and task results separately. The repository ships invariant tests, retrieval fixtures, and two development probes. Public and end-to-end benchmarks are planned.

Published results must include enough configuration and raw data to reproduce the run.

## Metrics

| Area | Measurements |
| --- | --- |
| Retrieval | Recall@k, first required rank, and forbidden-result count. |
| Validity | Memory lifecycle, Git applicability, and outcome classification. |
| Provenance | Coverage of required evidence and source references. |
| Context | Serialized bytes, token estimate, selected records, sources, and redundancy. |
| Performance | Capture, retrieval, and assembly latency; memory; database and index size. |
| Agent outcome | Validator success, tokens, calls, time, cost, repeated failures, and unsafe memory use. |

## Checks included in this repository

The repository includes checks for these storage and retrieval invariants:

- Replaying a stable source ID is idempotent, and derived records cannot reference missing evidence.
- Supersession changes the current view without deleting history.
- Scope filtering and kind, lifecycle, time, stale, and divergent eligibility run before channel limits. Divergent history is opt-in.
- The v1 migration rebuilds contentless FTS without losing search matches.
- Tied-score recall returns the same ordered IDs, signals, and score bit patterns.
- Snapshot v2 export, empty-target import, and re-export preserve canonical rows, SQLite `REAL` bit patterns, per-revision metadata, historical links, and the row-integrity footer; v1 imports remain supported.
- Batched hydration preserves the full order of evidence, artifacts, entities, and tags.
- Rendered text, structured sections, and ranked hits retain the same selected and truncated bodies under the reported token budget.
- Complete matching artifact sets can remain exact across unrelated dirty changes; partial automatic changed-file capture cannot establish exact applicability.
- Schema-v4 history retains source events, revision metadata fidelity, and immutable revision-scoped links. Legacy non-head metadata is explicitly incomplete.
- On Linux, macOS, and Windows, full-store purge removes the database, WAL, shared-memory, and rollback-journal files while leaving unrelated files, database or sidecar symlinks, user-controlled symlink or reparse-point paths, and multiply hard-linked paths untouched. The fixed macOS `/var` and `/tmp` system aliases are accepted.
- Scope-sensitive opening rejects a repository-local database or sidecar that is tracked, not ignored, reached through a symbolic link, reparse point, or redirected ancestor, or has a hard-link alias. External database names receive the same alias check.
- Workspaces sharing a repository cannot merge canonical identities or idempotency keys, revise each other's explicit IDs, attach each other's evidence or memory links, or overwrite each other's entity displays or artifact language metadata.

The [v1 fixture](../fixtures/eval/v1.jsonl) covers supersession, failures, repository isolation, branch divergence, stale artifacts, and exact errors. Its executable contract checks case structure, references, provenance, Git relationships, required and forbidden records, classifications, and the evidence behind relative-rank assertions:

~~~sh
node scripts/validate-eval-fixture.mjs
~~~

The script is deterministic and uses only the Node.js standard library. It validates the fixture's labeled retrieval intent; it is not a production-engine or end-to-end quality benchmark.

## Development probes

Ignored, fixed-workload probes cover core recall and diversity selection. They run in release mode with one thread and are profiling aids, not cross-machine claims. See [benchmark notes](../benches/README.md) for commands and equivalence checks.

## Planned evaluation

The following work is not yet a shipped benchmark suite.

### Public memory benchmarks

| Benchmark | Coverage |
| --- | --- |
| [SWE-ContextBench](https://arxiv.org/abs/2602.08316) | Software-engineering experience and harmful context. |
| [LongMemEval-V2](https://arxiv.org/abs/2605.12493) | Changing state, workflows, environment, and premise awareness. |
| [MemoryAgentBench](https://arxiv.org/abs/2507.05257) | Retrieval, test-time learning, long-range understanding, and forgetting. |
| [Memora / FAMA](https://arxiv.org/abs/2604.20006) | Mutation and obsolete-memory penalties. |
| [LongMemEval](https://arxiv.org/abs/2410.10813) | Multi-session and temporal reasoning, updates, extraction, and abstention. |

Adapters must preserve insertion order and must not expose answer labels during ingestion or retrieval.

### Git-state suite

A Git suite should cover descendant changes, divergent facts, reverts, cherry-picks, file/symbol renames, same-name symbol changes, dirty files, nested repositories, worktrees, and historical checkouts. The graph and explicit evidence provide ground truth, not an LLM judge.

### End-to-end agent comparison

Compare related coding tasks under four conditions:

1. No persistent memory.
2. Raw transcript or lexical history search.
3. A generic hybrid-retrieval baseline.
4. super-mem with the same model and context budget.

Use documented defaults for external systems and distinguish self-reported from reproduced results. Models, budgets, and retries must match.

Report validator success, tokens, calls, time, cost, code-region coverage, repeated failures, stale or forbidden use, and cited memories.

## Measurement protocol

### Context budgets

Use fixed budgets such as 512, 1,024, 2,048, and 4,096 tokens. Without the model tokenizer, report the deterministic estimate and bytes. Record candidates, selected records, sources, redundancy, and current, stale, divergent, contested, and failure section sizes. Label full-history comparisons; retrieval without context size can reward returning everything.

### Latency and resources

Measure capture and query separately at 1k, 10k, 100k, and, when practical, 1M deterministic events. Report cold/warm start, capture, retrieval, and assembly p50/p95/p99, rebuild throughput, resident memory, disk size, and single/concurrent-client behavior.

Keep optional model or embedding network latency separate from local database latency. Report synchronous and background work separately.

### Reproducibility

Every published run records:

- Commit and dirty state; compiler, lockfile, and build profile.
- Engine durability mode: `Balanced` (`synchronous=NORMAL`) or `Durable` (`synchronous=FULL`).
- OS, CPU, memory, storage, filesystem, and power mode.
- Corpus source, license, size, preprocessing, generator seed, and digest.
- Harness, model/version, sampling, prompts, embeddings, and reranking.
- Context budget, tokenizer, trials, warmup, concurrency, cache state, and cold/warm condition.
- Raw machine-readable results and evaluation code.

Use confidence intervals for stochastic agent experiments. A single successful run is an example, not a result.

### Leakage controls

- Keep fixture labels outside indexed payloads and separate development from held-out tasks.
- Do not tune ranking on the reported test set.
- Record query rewriting and model-assisted retrieval prompts.
- Use immutable benchmark snapshots and publish their digests.
- State whether candidate models or services may have trained on public answers.

## Failure analysis

Classify failures as ingestion gaps, scope-filtering errors, candidate misses, ranking misses, Git/temporal classification errors, budget exclusions, unsupported or misleading derived claims, or consuming-agent reasoning errors. This identifies whether capture, storage, retrieval, assembly, or agent behavior needs work.
