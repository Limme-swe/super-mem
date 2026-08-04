# Evaluation fixture format

[`v1.jsonl`](v1.jsonl) is a compact, hand-labeled retrieval fixture for core memory semantics. Each line is an independent case and must be valid JSON.

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

Records use only the fields needed by a case:

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

Production schemas may be richer. The fixture schema stays intentionally small so alternative implementations can consume it.

## Assertions

- `must_return`: records required in the bounded context.
- `must_not_return`: records forbidden by isolation or current-view semantics.
- `status`: expected lifecycle or outcome classification, such as current, superseded, validated-procedure, or known-failure.
- Git applicability uses the model-facing values `exact`, `compatible`, `stale`, `divergent`, `unversioned`, and `inapplicable`. `inapplicable` is a hard scope exclusion, not a low-ranked result.
- `rank_before`: ordered pairs `[preferred, lower_ranked]`.

A record may be absent from ordinary current-context output yet available through a historical/audit query. For example, `must_not_return` in the supersession case means “do not return as current evidence,” not “erase history.”

## Running an evaluator

An evaluator should:

1. Create an empty temporary store per case.
2. Insert records in listed order.
3. Configure the supplied repository graph/state without reading the labels.
4. Execute the query at its fixed budget.
5. Resolve returned record IDs and classifications.
6. Check all semantic assertions.
7. Report raw rankings and serialized context size.

The fixture is deliberately too small for performance or general quality claims. Its purpose is regression coverage for dangerous semantic failures.
