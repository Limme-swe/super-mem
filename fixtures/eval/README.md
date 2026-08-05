# Evaluation fixture format

[`v1.jsonl`](v1.jsonl) contains independent, hand-labeled retrieval cases. Each line is valid JSON.

## Shape

```json
{
  "schema_version": "supermem.eval.v1",
  "case_id": "stable-case-id",
  "labels": ["capability"],
  "description": "Human-readable intent",
  "repo_graph": {
    "repo_id": "repo identity",
    "parents": {"child_oid": ["parent_oid"]}
  },
  "records": [],
  "query": {
    "text": "query",
    "scope": {},
    "repo_state": {},
    "budget_tokens": 256
  },
  "expected": {
    "must_return": [],
    "must_not_return": [],
    "status": {},
    "rank_before": [],
    "reason": "Why these assertions matter"
  }
}
```

## Record fields

Each record uses only the fields needed by its case:

- `id`: stable fixture identifier.
- `kind`: fact, correction, decision, attempt, procedure, artifact, or diagnostic.
- `content`: payload visible to retrieval.
- `outcome`: success, failure, unknown, or not-applicable.
- `scope`: user/workspace/repository scope.
- `repo_state`: commit, branch, dirty digest, and linked artifacts.
- `observed_at`: ordering time.
- `supersedes`: older record ID when explicitly known.
- `provenance`: observable source type and identifier.
- `diagnostic_signature`: normalized exact-error key.

The production schema is richer; this smaller format is portable across evaluators.

## Assertions

- `must_return`: records required within the budget.
- `must_not_return`: records forbidden by isolation or current-view rules.
- `status`: expected lifecycle or outcome, such as current, superseded, validated-procedure, or known-failure.
- Git applicability uses the model-facing values `exact`, `compatible`, `stale`, `divergent`, `unversioned`, and `inapplicable`. `inapplicable` is a hard scope exclusion, not a low-ranked result.
- `rank_before`: ordered pairs `[preferred, lower_ranked]`.

A record can be absent from current context but available to a historical query. In the supersession case, `must_not_return` means “not current,” not “erase history.”

## Deterministic fixture regression

Run the checked-in contract test with Node.js:

```sh
node scripts/validate-eval-fixture.mjs
```

The script uses only the Node.js standard library. It checks case and record references, provenance, repository-graph integrity, supersession order, repository isolation, Git ancestry and divergence, changed artifact and symbol digests, exact diagnostic signals, required and forbidden results, and the evidence behind relative-rank assertions. CI runs the same command.

This is a deterministic regression for the fixture and its labeled retrieval intent. It does not run the production engine or claim retrieval-quality measurements; a production evaluator must also follow the contract below.

## Evaluator contract

For each case, an evaluator must:

1. Create an empty temporary store per case.
2. Insert records in listed order.
3. Configure the supplied repository graph and state without reading labels.
4. Execute the query at its fixed budget.
5. Resolve returned IDs and classifications.
6. Check all semantic assertions.
7. Report raw ranks and serialized context size.

This fixture checks semantic regressions; it is not a performance or general-quality benchmark.
