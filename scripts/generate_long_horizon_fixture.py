#!/usr/bin/env python3
"""Generate the deterministic long-horizon retrieval benchmark fixture."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OUTPUT = ROOT / "fixtures" / "long-horizon"
CASE_SCHEMA = "supermem.retrieval.case.v1"
QREL_SCHEMA = "supermem.retrieval.qrels.v1"
NAMESPACE = "long-horizon-v1"
AS_OF = "2026-08-15T12:00:00Z"


def memory_id(case_number: int, item_number: int) -> str:
    return f"018f2000-{case_number:04x}-7000-8000-{item_number:012x}"


def scope(
    repo: str,
    *,
    workspace: str = "workspace-a",
    branch: str = "main",
    head: str | None = None,
    session: str | None = None,
) -> dict[str, Any]:
    repository: dict[str, Any] = {
        "repo_id": repo,
        "branch": branch,
        "head_oid": head or hashlib.sha1(f"{repo}\0{branch}".encode(), usedforsecurity=False).hexdigest(),
    }
    result: dict[str, Any] = {
        "namespace": NAMESPACE,
        "workspace_id": workspace,
        "repository": repository,
    }
    if session is not None:
        result["session_id"] = session
    return result


def artifact(repo: str, path: str, symbol: str, content_hash: str) -> dict[str, Any]:
    return {
        "repo_id": repo,
        "path": path,
        "symbol": symbol,
        "content_hash": content_hash,
        "git_oid": None,
        "language": "rust",
    }


def remember(
    *,
    case_id: str,
    record_id: str,
    case_number: int,
    item_number: int,
    kind: str,
    memory_scope: dict[str, Any],
    title: str,
    body: str,
    importance: float = 0.7,
    confidence: float = 0.9,
    trust: str = "tool_verified",
    canonical_key: str | None = None,
    attributes: dict[str, Any] | None = None,
    artifacts: list[dict[str, Any]] | None = None,
    tags: list[str] | None = None,
) -> dict[str, Any]:
    request: dict[str, Any] = {
        "memory_id": memory_id(case_number, item_number),
        "idempotency_key": f"fixture:{case_id}:{record_id}",
        "kind": kind,
        "scope": memory_scope,
        "canonical_key": canonical_key,
        "title": title,
        "body": body,
        "importance": importance,
        "confidence": confidence,
        "trust": trust,
        "valid_from": None,
        "valid_until": None,
        "expires_at": None,
        "attributes": attributes or {},
        "tags": tags or [],
        "entities": [],
        "artifacts": artifacts or [],
        "evidence": [],
        "links": [],
    }
    return {"op": "remember", "record_id": record_id, "request": request}


def recall(
    query: str,
    recall_scope: dict[str, Any],
    *,
    artifacts: list[dict[str, Any]] | None = None,
    error_fingerprint: str | None = None,
    token_budget: int = 768,
) -> dict[str, Any]:
    return {
        "query": query,
        "scope": recall_scope,
        "limit": 10,
        "token_budget": token_budget,
        "kinds": [],
        "as_of": AS_OF,
        "include_stale": False,
        "include_divergent": False,
        "include_superseded": False,
        "hints": {
            "artifacts": artifacts or [],
            "error_fingerprint": error_fingerprint,
            "entities": [],
        },
    }


def distractors(case_id: str, case_number: int, repo: str, start: int = 20) -> list[dict[str, Any]]:
    topics = [
        ("Cache eviction policy", "The thumbnail cache uses a clock sweep after 4096 entries."),
        ("Metrics transport", "Operational counters are exported through the metrics sidecar."),
        ("Temporary directory", "Integration tests allocate temporary directories per process."),
        ("Request tracing", "Trace identifiers are propagated through outbound request headers."),
        ("Dependency policy", "Workspace dependencies are declared at the root manifest."),
        ("Release archive", "Release archives contain the binary, adapters, and licenses."),
        ("Retry delay", "Background retries use bounded exponential backoff with jitter."),
        ("Log retention", "Debug logs are rotated after seven local files."),
    ]
    return [
        remember(
            case_id=case_id,
            record_id=f"noise-{index}",
            case_number=case_number,
            item_number=start + index,
            kind="fact",
            memory_scope=scope(repo, session=f"noise-session-{index}"),
            title=title,
            body=body,
            importance=0.25,
            confidence=0.65,
            trust="agent",
        )
        for index, (title, body) in enumerate(topics, start=1)
    ]


def make_case(
    case_id: str,
    case_number: int,
    operations: list[dict[str, Any]],
    recall_request: dict[str, Any],
    *,
    relevance: dict[str, int],
    forbidden: list[str] | None = None,
    rank_before: list[list[str]] | None = None,
    expected_signals: dict[str, list[str]] | None = None,
    expected_applicability: dict[str, str] | None = None,
    expected_excluded: dict[str, str] | None = None,
    expected_revision: dict[str, int] | None = None,
    forbidden_body_substrings: list[str] | None = None,
) -> tuple[dict[str, Any], dict[str, Any]]:
    case = {
        "schema_version": CASE_SCHEMA,
        "case_id": case_id,
        "operations": operations,
        "recall": recall_request,
    }
    qrel: dict[str, Any] = {
        "schema_version": QREL_SCHEMA,
        "case_id": case_id,
        "relevance": relevance,
        "forbidden": forbidden or [],
        "rank_before": rank_before or [],
    }
    optional = {
        "expected_signals": expected_signals,
        "expected_applicability": expected_applicability,
        "expected_excluded": expected_excluded,
        "expected_revision": expected_revision,
        "forbidden_body_substrings": forbidden_body_substrings,
    }
    for key, value in optional.items():
        if value:
            qrel[key] = value
    return case, qrel


def generate() -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    cases: list[dict[str, Any]] = []
    qrels: list[dict[str, Any]] = []
    case_number = 1

    decision_data = [
        ("quartz", "queue ownership", "The scheduler owns retry queues; workers only acknowledge completed leases.", "Who owns retry queues after the lease refactor?"),
        ("cedar", "configuration precedence", "Command-line flags override environment variables, which override platform defaults.", "Which configuration source wins when both a flag and environment value exist?"),
        ("ember", "schema migration", "Schema migrations run only through normal initialization, never through observational diagnostics.", "Which command path is allowed to migrate the store?"),
        ("harbor", "release profile", "The optimized Cargo profile belongs in the workspace root rather than a member crate.", "Where must the optimized Cargo profile be declared?"),
    ]
    for slug, title, body, query in decision_data:
        cid = f"multi-session-decision-{slug}"
        repo = f"repo:{slug}"
        target = memory_id(case_number, 1)
        lower = memory_id(case_number, 2)
        operations = [
            remember(
                case_id=cid,
                record_id="target-decision",
                case_number=case_number,
                item_number=1,
                kind="decision",
                memory_scope=scope(repo, session="architecture-session"),
                title=title.title(),
                body=body,
                importance=0.95,
                confidence=0.98,
                trust="user_confirmed",
                canonical_key=f"decision:{slug}",
            ),
            remember(
                case_id=cid,
                record_id="plausible-distractor",
                case_number=case_number,
                item_number=2,
                kind="fact",
                memory_scope=scope(repo, session="old-session"),
                title=f"Historical {title}",
                body=f"An early prototype documented a different {title}, but it was never adopted.",
                importance=0.35,
                confidence=0.45,
                trust="agent",
            ),
            *distractors(cid, case_number, repo),
        ]
        case, qrel = make_case(
            cid,
            case_number,
            operations,
            recall(query, scope(repo, session="new-session")),
            relevance={target: 3, lower: 0, **{memory_id(case_number, 20 + i): 0 for i in range(1, 9)}},
            rank_before=[[target, lower]],
        )
        cases.append(case); qrels.append(qrel); case_number += 1

    procedure_data = [
        ("wal", "close a live WAL", "Stop every writer, checkpoint the owner, then run doctor against the stable store.", "How should a live WAL be handled before diagnosis?"),
        ("lock", "release an async lock", "Move the guard into an inner block so it is dropped before the await point.", "How do we make the future Send when a lock guard exists?"),
        ("snapshot", "verify a restore", "Import into a separate empty database, run status and doctor, then perform representative recall.", "How do we safely test whether a memory backup restores?"),
        ("adapter", "debug silent capture", "First prove CLI write and recall, then test MCP, then automatic lifecycle capture in that order.", "What order isolates a silent harness integration?"),
    ]
    for slug, title, procedure_body, query in procedure_data:
        cid = f"procedure-over-failed-attempt-{slug}"
        repo = f"repo:procedure-{slug}"
        target = memory_id(case_number, 1)
        failed = memory_id(case_number, 2)
        operations = [
            remember(
                case_id=cid,
                record_id="verified-procedure",
                case_number=case_number,
                item_number=1,
                kind="procedure",
                memory_scope=scope(repo, session="resolution-session"),
                title=title.title(),
                body=procedure_body,
                importance=0.92,
                confidence=0.96,
                trust="tool_verified",
                attributes={"succeeded": True, "verification": "focused regression passed"},
            ),
            remember(
                case_id=cid,
                record_id="failed-attempt",
                case_number=case_number,
                item_number=2,
                kind="outcome",
                memory_scope=scope(repo, session="failed-session"),
                title=f"Failed attempt to {title}",
                body=f"Attempted to {title} by forcing the first obvious command. It failed and should not be reused.",
                importance=0.25,
                confidence=0.9,
                trust="tool_verified",
                attributes={"succeeded": False, "promotion_reason": "failed_execution"},
            ),
            *distractors(cid, case_number, repo),
        ]
        case, qrel = make_case(
            cid,
            case_number,
            operations,
            recall(query, scope(repo, session="later-session")),
            relevance={target: 3, failed: 1, **{memory_id(case_number, 20 + i): 0 for i in range(1, 9)}},
            rank_before=[[target, failed]],
        )
        cases.append(case); qrels.append(qrel); case_number += 1

    revision_data = [
        ("msrv", "compiler floor", "Rust 1.88", "Rust 1.82", "What is the oldest compiler accepted now?"),
        ("endpoint", "create endpoint", "POST /api/v3/orders", "POST /api/v2/orders", "Which endpoint creates orders now?"),
        ("timeout", "writer timeout", "8000 milliseconds", "3000 milliseconds", "What is the current harness timeout?"),
        ("storage", "canonical storage", "local SQLite", "JSON files", "What is the canonical memory store?"),
    ]
    for slug, title, current, retired, query in revision_data:
        cid = f"current-revision-{slug}"
        repo = f"repo:revision-{slug}"
        target = memory_id(case_number, 1)
        distractor = memory_id(case_number, 2)
        operations = [
            remember(
                case_id=cid,
                record_id="revision-one",
                case_number=case_number,
                item_number=1,
                kind="constraint",
                memory_scope=scope(repo, session="old-session"),
                title=title.title(),
                body=f"The {title} is {retired}.",
                importance=0.9,
                confidence=0.95,
                trust="user_confirmed",
                canonical_key=f"current:{slug}",
            ),
            remember(
                case_id=cid,
                record_id="revision-two",
                case_number=case_number,
                item_number=1,
                kind="constraint",
                memory_scope=scope(repo, session="migration-session"),
                title=title.title(),
                body=f"The current {title} is {current}.",
                importance=0.9,
                confidence=0.98,
                trust="user_confirmed",
                canonical_key=f"current:{slug}",
            ),
            remember(
                case_id=cid,
                record_id="other-constraint",
                case_number=case_number,
                item_number=2,
                kind="constraint",
                memory_scope=scope(repo),
                title="Unrelated constraint",
                body="The release archives must include third-party license notices.",
                importance=0.7,
                confidence=0.9,
                trust="user_confirmed",
            ),
            *distractors(cid, case_number, repo),
        ]
        case, qrel = make_case(
            cid,
            case_number,
            operations,
            recall(query, scope(repo, session="current-session")),
            relevance={target: 3, distractor: 0, **{memory_id(case_number, 20 + i): 0 for i in range(1, 9)}},
            expected_revision={target: 2},
            forbidden_body_substrings=[retired],
        )
        cases.append(case); qrels.append(qrel); case_number += 1

    isolation_data = [
        ("auth", "Authentication tokens are validated by the gateway middleware.", "Where are authentication tokens validated?"),
        ("billing", "Invoice retries are scheduled by the billing worker.", "Which component schedules invoice retries?"),
        ("search", "Search aliases are rebuilt from canonical memory rows.", "What source rebuilds search aliases?"),
        ("release", "Release checksums are generated after every native archive exists.", "When are release checksums generated?"),
    ]
    for slug, body, query in isolation_data:
        cid = f"repository-isolation-{slug}"
        repo = f"repo:isolation-{slug}"
        target = memory_id(case_number, 1)
        forbidden = memory_id(case_number, 2)
        operations = [
            remember(
                case_id=cid,
                record_id="authorized-repository",
                case_number=case_number,
                item_number=1,
                kind="fact",
                memory_scope=scope(repo, session="repo-session"),
                title=f"{slug.title()} ownership",
                body=body,
                importance=0.85,
                confidence=0.95,
                trust="tool_verified",
            ),
            remember(
                case_id=cid,
                record_id="other-repository",
                case_number=case_number,
                item_number=2,
                kind="fact",
                memory_scope=scope(f"repo:other-{slug}", session="foreign-session"),
                title=f"{slug.title()} ownership",
                body=body.replace("gateway", "frontend").replace("billing worker", "web process").replace("canonical memory rows", "remote cache").replace("every native archive exists", "the first archive exists"),
                importance=1.0,
                confidence=1.0,
                trust="user_confirmed",
            ),
            *distractors(cid, case_number, repo),
        ]
        relevance = {target: 3, forbidden: 0, **{memory_id(case_number, 20 + i): 0 for i in range(1, 9)}}
        case, qrel = make_case(
            cid,
            case_number,
            operations,
            recall(query, scope(repo, session="new-session")),
            relevance=relevance,
            forbidden=[forbidden],
            expected_excluded={forbidden: "inapplicable"},
        )
        cases.append(case); qrels.append(qrel); case_number += 1

    for index, slug in enumerate(["cache", "deploy", "schema", "alerts"], start=1):
        cid = f"workspace-isolation-{slug}"
        repo = f"repo:workspace-{slug}"
        target = memory_id(case_number, 1)
        forbidden = memory_id(case_number, 2)
        body = f"The {slug} workflow for workspace alpha uses the verified alpha procedure {index}."
        operations = [
            remember(
                case_id=cid,
                record_id="workspace-alpha",
                case_number=case_number,
                item_number=1,
                kind="procedure",
                memory_scope=scope(repo, workspace="workspace-alpha", session="alpha-session"),
                title=f"{slug.title()} workflow",
                body=body,
                importance=0.85,
                confidence=0.95,
                trust="tool_verified",
            ),
            remember(
                case_id=cid,
                record_id="workspace-beta",
                case_number=case_number,
                item_number=2,
                kind="procedure",
                memory_scope=scope(repo, workspace="workspace-beta", session="beta-session"),
                title=f"{slug.title()} workflow",
                body=f"The {slug} workflow for workspace beta uses an incompatible beta-only procedure.",
                importance=1.0,
                confidence=1.0,
                trust="user_confirmed",
            ),
            *[
                remember(
                    case_id=cid,
                    record_id=f"alpha-noise-{noise}",
                    case_number=case_number,
                    item_number=20 + noise,
                    kind="fact",
                    memory_scope=scope(repo, workspace="workspace-alpha", session=f"noise-{noise}"),
                    title=f"Alpha noise {noise}",
                    body=f"Unrelated alpha workspace note number {noise}.",
                    importance=0.2,
                    confidence=0.6,
                    trust="agent",
                )
                for noise in range(1, 9)
            ],
        ]
        case, qrel = make_case(
            cid,
            case_number,
            operations,
            recall(f"What is the {slug} workflow for workspace alpha?", scope(repo, workspace="workspace-alpha", session="current")),
            relevance={target: 3, forbidden: 0, **{memory_id(case_number, 20 + i): 0 for i in range(1, 9)}},
            forbidden=[forbidden],
            expected_excluded={forbidden: "inapplicable"},
        )
        cases.append(case); qrels.append(qrel); case_number += 1

    stale_data = [
        ("router", "src/router.rs", "Router::dispatch", "The current transport is registered through the transport registry."),
        ("config", "src/config.rs", "Config::load", "The current configuration loader merges flags after environment values."),
        ("worker", "src/worker.rs", "Worker::run", "The current worker acknowledges a lease only after persistence succeeds."),
        ("snapshot", "src/snapshot.rs", "Snapshot::verify", "The current snapshot verifier checks the footer digest before import."),
    ]
    for slug, path, symbol, current_body in stale_data:
        cid = f"artifact-freshness-{slug}"
        repo = f"repo:artifact-{slug}"
        stale_id = memory_id(case_number, 1)
        current_id = memory_id(case_number, 2)
        stale_hash = ("1" * 63) + f"{case_number % 16:x}"
        current_hash = ("2" * 63) + f"{case_number % 16:x}"
        operations = [
            remember(
                case_id=cid,
                record_id="stale-memory",
                case_number=case_number,
                item_number=1,
                kind="procedure",
                memory_scope=scope(repo, head="a" * 40, session="old-session"),
                title=f"Old {slug} procedure",
                body=f"An obsolete {slug} procedure edits {symbol} directly.",
                artifacts=[artifact(repo, path, symbol, stale_hash)],
                importance=0.9,
                confidence=0.95,
                trust="tool_verified",
            ),
            remember(
                case_id=cid,
                record_id="current-memory",
                case_number=case_number,
                item_number=2,
                kind="procedure",
                memory_scope=scope(repo, head="b" * 40, session="current-session"),
                title=f"Current {slug} procedure",
                body=current_body,
                artifacts=[artifact(repo, path, symbol, current_hash)],
                importance=0.9,
                confidence=0.96,
                trust="tool_verified",
            ),
            *distractors(cid, case_number, repo),
        ]
        hint = artifact(repo, path, symbol, current_hash)
        case, qrel = make_case(
            cid,
            case_number,
            operations,
            recall(f"What is the current {slug} procedure around {symbol}?", scope(repo, head="b" * 40, session="later"), artifacts=[hint]),
            relevance={stale_id: 0, current_id: 3, **{memory_id(case_number, 20 + i): 0 for i in range(1, 9)}},
            forbidden=[stale_id],
            expected_applicability={current_id: "exact"},
            expected_excluded={stale_id: "stale"},
        )
        cases.append(case); qrels.append(qrel); case_number += 1

    error_data = [
        ("e0277", "smerr1:e0277-send-guard", "E0277 future cannot be sent between threads safely", "Drop the non-Send guard before awaiting."),
        ("sqlite-busy", "smerr1:sqlite-busy-writer", "database is locked during checkpoint", "Keep the hook timeout above the SQLite busy timeout."),
        ("address-use", "smerr1:address-in-use", "address already in use on port 8080", "Release the previous listener before rebinding the port."),
        ("schema-old", "smerr1:schema-old", "database migration required", "Open through init after taking a logical export."),
    ]
    for slug, fingerprint, error_text, resolution in error_data:
        cid = f"diagnostic-fingerprint-{slug}"
        repo = f"repo:error-{slug}"
        target = memory_id(case_number, 1)
        lower = memory_id(case_number, 2)
        operations = [
            remember(
                case_id=cid,
                record_id="diagnostic-resolution",
                case_number=case_number,
                item_number=1,
                kind="procedure",
                memory_scope=scope(repo, session="failure-resolution"),
                title=f"Resolve {slug}",
                body=f"Observed {error_text}. {resolution}",
                attributes={"error_fingerprint": fingerprint, "succeeded": True},
                importance=0.9,
                confidence=0.95,
                trust="tool_verified",
            ),
            remember(
                case_id=cid,
                record_id="generic-error-note",
                case_number=case_number,
                item_number=2,
                kind="fact",
                memory_scope=scope(repo, session="generic"),
                title="Generic error note",
                body="Errors should be inspected before retrying a command.",
                importance=0.4,
                confidence=0.7,
                trust="agent",
            ),
            *distractors(cid, case_number, repo),
        ]
        case, qrel = make_case(
            cid,
            case_number,
            operations,
            recall(f"How was this failure resolved: {error_text}?", scope(repo, session="new"), error_fingerprint=fingerprint),
            relevance={target: 3, lower: 0, **{memory_id(case_number, 20 + i): 0 for i in range(1, 9)}},
            rank_before=[[target, lower]],
            expected_signals={target: ["error_fingerprint"]},
        )
        cases.append(case); qrels.append(qrel); case_number += 1

    alias_data = [
        ("snapshot", "finish_snapshot_import", "SnapshotImport::finish", "Call finish_snapshot_import only after the footer digest is verified.", "How do we finish snapshot import after checking the footer?"),
        ("account", "account_address", "AccountAddress", "Normalize account_address before constructing AccountAddress.", "How should the account address value be normalized?"),
        ("writer", "writer_lock_probe", "WriterLockProbe", "Run writer_lock_probe before opening the diagnostic copy.", "What probe runs before the diagnostic copy is opened?"),
        ("artifact", "artifact_projection_status", "ArtifactProjectionStatus", "Inspect artifact_projection_status before rebuilding derived fingerprints.", "What status should be inspected before rebuilding artifact fingerprints?"),
    ]
    for slug, snake, camel, body, query in alias_data:
        cid = f"code-alias-{slug}"
        repo = f"repo:alias-{slug}"
        target = memory_id(case_number, 1)
        lower = memory_id(case_number, 2)
        operations = [
            remember(
                case_id=cid,
                record_id="alias-target",
                case_number=case_number,
                item_number=1,
                kind="procedure",
                memory_scope=scope(repo, session="implementation"),
                title=camel,
                body=body,
                artifacts=[artifact(repo, f"src/{slug}.rs", camel, "3" * 64)],
                importance=0.85,
                confidence=0.95,
                trust="tool_verified",
                tags=[snake],
            ),
            remember(
                case_id=cid,
                record_id="alias-distractor",
                case_number=case_number,
                item_number=2,
                kind="fact",
                memory_scope=scope(repo, session="docs"),
                title=f"{slug.title()} overview",
                body=f"The {slug} module contains several public helper types.",
                importance=0.35,
                confidence=0.7,
                trust="agent",
            ),
            *distractors(cid, case_number, repo),
        ]
        case, qrel = make_case(
            cid,
            case_number,
            operations,
            recall(query, scope(repo, session="later")),
            relevance={target: 3, lower: 0, **{memory_id(case_number, 20 + i): 0 for i in range(1, 9)}},
            rank_before=[[target, lower]],
            expected_signals={target: ["code_alias"]},
        )
        cases.append(case); qrels.append(qrel); case_number += 1

    assert len(cases) == 32
    return cases, qrels


def encoded_lines(rows: list[dict[str, Any]]) -> str:
    return "".join(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n" for row in rows)


def write_or_check(output: Path, check: bool) -> int:
    cases, qrels = generate()
    expected = {
        output / "v1.jsonl": encoded_lines(cases),
        output / "qrels-v1.jsonl": encoded_lines(qrels),
    }
    if check:
        failed = False
        for path, content in expected.items():
            actual = path.read_text(encoding="utf-8") if path.exists() else None
            if actual != content:
                print(f"out of date: {path.relative_to(ROOT)}", file=sys.stderr)
                failed = True
        return 1 if failed else 0
    output.mkdir(parents=True, exist_ok=True)
    for path, content in expected.items():
        path.write_text(content, encoding="utf-8")
    print(f"generated {len(cases)} long-horizon cases in {output}")
    return 0


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument("--check", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    return write_or_check(args.output.resolve(), args.check)


if __name__ == "__main__":
    raise SystemExit(main())
