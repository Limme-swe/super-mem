# Evaluation methodology

Memory quality is not one number. `super-mem` evaluates selection, temporal and Git validity, contradiction handling, provenance, context cost, and downstream utility separately.

No benchmark result should be published without enough configuration to reproduce it.

## Evaluation questions

1. Does retrieval include the evidence needed for the task?
2. Does it exclude evidence from the wrong user, repository, or branch?
3. Does it distinguish current, superseded, stale, and divergent knowledge?
4. Does it distinguish successful procedures from failed attempts?
5. Can every derived result be traced to observable evidence?
6. How much context does it consume?
7. How long does capture, retrieval, and context assembly take?
8. Does the context improve an agent's coding outcome under a fixed budget?

## Levels of evaluation

### 1. Record and invariant tests

Test durable and derived-state invariants:

- Event append is idempotent for a stable source ID.
- A derived record cannot reference missing evidence.
- Supersession preserves history while changing the current view.
- On supported Unix hosts, full-store purge removes the database plus its WAL and shared-memory sidecars; tests also verify that unrelated, symbolic-link, and multiply hard-linked paths are untouched. V0.1 refuses purge on Windows.
- Scope filters apply before result ranking.
- Projection rebuild produces the same logical results.
- Crash recovery never acknowledges a missing event.
- Repeated tied-score recall returns identical ordered IDs, signals, and score
  bit patterns.
- Export, empty-target import, and re-export preserve canonical rows, SQLite
  REAL bit patterns, and the snapshot row-integrity footer.
- Batched hydration preserves the full deterministic order of evidence,
  artifacts, entities, and tags.
- Scope-sensitive opening rejects a repository-local main file or sidecar that
  could mutate tracked Git state through tracking, missing ignore rules, a
  symbolic link or symlinked ancestor, or a hard-link alias.
- Workspaces sharing a repository cannot coalesce canonical identities or
  idempotency keys, revise each other's explicit IDs, attach each other's
  evidence or memory links, or overwrite each other's entity displays or
  artifact language metadata.

### 2. Retrieval fixtures

The labeled [v1 fixture](../fixtures/eval/v1.jsonl) isolates six failure modes:

- Supersession.
- Failed attempts.
- Repository isolation.
- Branch divergence.
- Stale artifacts.
- Exact error recall.

For each case, report:

- Recall@k for required records.
- Forbidden-result count.
- Expected status/classification accuracy.
- Rank of the first required result.
- Evidence/provenance coverage.
- Serialized context size.

Passing a fixture requires satisfying its semantic assertions, not merely returning a string match.

### 3. Public memory benchmarks

Use public benchmarks for complementary capabilities:

| Benchmark | What it contributes |
| --- | --- |
| [SWE-ContextBench](https://arxiv.org/abs/2602.08316) | Reuse of related software-engineering experience and sensitivity to incorrectly selected context. |
| [LongMemEval-V2](https://arxiv.org/abs/2605.12493) | Static and dynamic state, workflow knowledge, environment gotchas, and premise awareness over agent trajectories. |
| [MemoryAgentBench](https://arxiv.org/abs/2507.05257) | Retrieval, test-time learning, long-range understanding, and selective forgetting. |
| [Memora / FAMA](https://arxiv.org/abs/2604.20006) | Consolidation under repeated mutation and penalties for using obsolete memories. |
| [LongMemEval](https://arxiv.org/abs/2410.10813) | Information extraction, multi-session reasoning, temporal reasoning, updates, and abstention. |

Benchmark adapters must preserve the benchmark's insertion order and must not use answer labels during ingestion or retrieval.

### 4. Git-state suite

Public conversational benchmarks do not fully test repository history. Maintain a reproducible suite with real temporary repositories covering:

- A fact changed on a descendant commit.
- Two valid facts on divergent branches.
- A reverted change.
- A cherry-pick into another branch.
- File rename with unchanged symbol.
- Symbol rename with similar implementation.
- Changed symbol retaining the same name.
- Dirty tracked and untracked worktree changes.
- Nested repositories and worktrees.
- Historical query against an older checkout.

The ground truth is the repository graph and explicit evidence, not an LLM judge.

### 5. End-to-end agent evaluation

For related coding tasks, compare at least:

1. No persistent memory.
2. Raw transcript or lexical history search.
3. A generic hybrid retrieval baseline.
4. `super-mem` with the same model and context budget.

Where practical, add external memory systems using their documented default configurations, clearly labeling self-reported versus reproduced results.

Measure:

- Resolved task rate under the benchmark's validator.
- Tokens, tool calls, wall-clock time, and model cost.
- Relevant code-region coverage.
- Repeated failed actions.
- Stale or forbidden memory use.
- Memory citations used in the final action or explanation.

Do not give `super-mem` more context tokens, retries, or a stronger model than baselines.

## Context-budget protocol

Report fixed budgets, for example 512, 1,024, 2,048, and 4,096 model tokens. If an exact model tokenizer is unavailable, report the deterministic approximation and serialized byte count.

Context assembly measurements should include:

- Raw candidate count.
- Selected record count.
- Evidence-source count.
- Duplicate/redundant fraction.
- Current, stale, divergent, contested, and failure section sizes.

Retrieval-only metrics without context size can reward systems that return nearly the entire history. Full-history comparisons should be labeled separately.

## Latency and resource protocol

Measure capture and query paths independently at corpus sizes such as 1k, 10k, 100k, and—when practical—1M events.

Report:

- Cold-start and warm-start time.
- Capture acknowledgment p50/p95/p99.
- Retrieval p50/p95/p99.
- End-to-end context assembly p50/p95/p99.
- Index/rebuild throughput.
- Peak and steady resident memory.
- On-disk event and index size.
- Single-client and concurrent-agent behavior.

Do not mix optional model/embedding network latency into local database latency. Report synchronous and background work separately.

The repository includes ignored, fixed-workload development probes for the
core recall pipeline and the diversity selector. Run them in release mode and
with one test thread; they are profiling aids rather than cross-machine product
claims. See [benchmark notes](../benches/README.md) for exact commands.

## Reproducibility record

Every published run should include:

- `super-mem` commit and dirty status.
- Rust/compiler and dependency lockfile versions.
- OS, CPU, memory, storage, and power mode.
- Corpus source, license, size, and preprocessing.
- Harness, model, model version, sampling settings, and prompts.
- Embedding and reranking configuration, if enabled.
- Context budget and tokenizer.
- Number of trials, warmup, concurrency, and cache state.
- Raw machine-readable results and evaluation code.

Use confidence intervals for stochastic end-to-end experiments. A single successful agent run is an example, not a result.

## Preventing benchmark leakage

- Keep fixture labels outside indexed memory payloads.
- Separate development and held-out repository/task sequences.
- Do not tune ranking weights on the reported test set.
- Record all query rewriting and model-assisted retrieval prompts.
- Use immutable benchmark snapshots and publish their digests.
- Check whether candidate models or memory services may have trained on public answers, and state that limitation.

## Interpreting failures

Classify errors rather than collapsing them into answer accuracy:

- Missing evidence at ingestion.
- Scope filtering error.
- Candidate-generation miss.
- Ranking miss.
- Git/temporal classification error.
- Context-budget exclusion.
- Unsupported or misleading derived claim.
- Reader/agent reasoning error despite correct evidence.

This separation identifies whether to improve capture, storage, retrieval, context assembly, or the consuming agent.
