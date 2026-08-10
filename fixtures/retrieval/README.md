# Retrieval quality starter fixture

This directory contains a small, executable-oriented retrieval corpus for the production memory types. It is separate from [`fixtures/eval`](../eval), whose portable cases describe semantic invariants but do not directly drive `MemoryEngine`.

The starter fixture is a regression seed, not a representative benchmark. Twelve cases are too few to support a general retrieval-quality claim.

## Files

- [`v1.jsonl`](v1.jsonl) contains ingestion operations and recall requests.
- [`qrels-v1.jsonl`](qrels-v1.jsonl) contains relevance grades and safety assertions. Qrels must never be passed to the engine, encoder, query rewriter, or indexer.

Each line of `v1.jsonl` is one independent case. A runner must create a fresh temporary store for the case, execute `operations` in order, and then pass `recall` directly to `MemoryEngine::recall`.

The object under an operation's `request` uses the serialized `RememberRequest` field names and values. `record_id` is runner metadata only. It must not be copied into a memory title, body, attributes, tags, entities, or artifacts. Explicit UUIDs keep result comparison and tie handling deterministic. Repeated UUIDs deliberately create revisions, as in `revision-correction-msrv`.

The `recall` object uses the serialized `RecallRequest` shape. Times, limits, budgets, scopes, repository states, and hints are fixed by the fixture. A runner must not add qrel-derived tags, hints, filters, or query text.

Validate the two files structurally with:

```sh
node scripts/validate-retrieval-fixture.mjs
```

`cargo test -p super-mem-core --test retrieval_fixture -- --nocapture` then executes the same writes and recalls through the production `MemoryEngine`. It hard-fails safety, current-revision, exact-signal, applicability, and selected exact/alias rank assertions. It prints MRR@10, Recall@10, and nDCG@10 diagnostically without treating this small starter corpus as a semantic-quality threshold.

## Qrels

Relevance grades have these meanings:

| Grade | Meaning |
| --- | --- |
| 3 | Essential answer. |
| 2 | Useful supporting evidence or warning. |
| 1 | Relevant historical context. |
| 0 | Irrelevant to the requested answer. |

Missing IDs are treated as grade 0. `forbidden` is an independent safety assertion: a forbidden ID must not occur in ranked hits, structured sections, or rendered context, regardless of its relevance grade.

Optional assertions are:

- `rank_before`: the first ID must rank ahead of the second.
- `expected_signals`: required `RetrievalSignal` values on a returned hit.
- `expected_applicability`: applicability for a returned hit.
- `expected_excluded`: the classification that explains a hard exclusion.
- `expected_revision`: the returned head revision.
- `forbidden_body_substrings`: retired content that must not survive in the current result.

Checking `expected_excluded` requires a test-only retrieval trace or a direct applicability check. It must not be implemented by exposing inaccessible records through the public context pack.

## Metrics

Report metrics from final, budgeted `ContextPack.hits`, not an unbounded internal candidate list:

- **MRR@10:** reciprocal rank of the first grade-3 result, averaged by case.
- **Recall@k:** fraction of grade-2-or-higher IDs returned at `k`.
- **nDCG@10:** graded gain `2^grade - 1`, with deterministic ID order for score ties.
- **Forbidden@10:** total forbidden IDs returned in the first ten hits. This must be zero.

A benchmark report intended to support a quality claim should also include every case's raw ordered IDs, relevance grades, signals, applicability, revision, estimated tokens, and rendered byte count. The checked-in regression runner prints only aggregate diagnostics and hard-fails its explicit safety assertions; it is not a publishing harness. Do not hide a failed safety assertion inside an aggregate score.

For a hybrid semantic change, run the same canonical cases in at least these modes:

1. Existing exact, lexical, structured, and recency channels with semantic retrieval disabled.
2. Hybrid retrieval using an exact vector scan.
3. Hybrid retrieval using the proposed vector index.

The exact scan separates embedding and fusion quality from approximate-index loss. Any approximate implementation should additionally report neighbor Recall@k against that scan.

## Leakage and tuning rules

- Qrels and their field names stay outside all indexed or embedded payloads.
- Do not derive tags, entities, artifact hints, titles, or query rewrites from qrels.
- Do not train or tune a model, signal weight, ANN parameter, or rewrite prompt on these cases and then report the same cases as held out.
- Treat all variants of one case as a single split group. A later paraphrase of a case cannot be placed in a different train/test split.
- Keep exact identifiers, error fingerprints, repository IDs, workspace IDs, artifact hashes, and lifecycle state as legitimate production signals; do not duplicate them into hidden evaluator-only text.
- Record the fixture digest, model and tokenizer digests, vector precision, index parameters, build commit, and generator seed with results.
- Freeze a substantially larger held-out suite before publishing quality claims. Include independently authored queries, balanced metadata, crowdout cases, and deterministic distractor corpora.

The fixture intentionally contains exact-channel regressions alongside weak-overlap paraphrases. A semantic layer is additive: it must improve difficult queries without weakening identifiers, diagnostics, scope isolation, Git applicability, revision correctness, or token budgeting.
