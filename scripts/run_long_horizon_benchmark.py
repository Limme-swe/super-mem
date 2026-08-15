#!/usr/bin/env python3
"""Run the production long-horizon retrieval benchmark and validate its report."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SCHEMA = "supermem.long_horizon.report.v1"


def load_report(path: Path) -> dict[str, Any]:
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"could not read benchmark report {path}: {error}") from error
    if not isinstance(payload, dict) or payload.get("schema_version") != SCHEMA:
        raise ValueError("benchmark report has an unsupported schema")
    metrics = payload.get("metrics")
    cases = payload.get("cases")
    if not isinstance(metrics, dict) or not isinstance(cases, list):
        raise ValueError("benchmark report is missing metrics or cases")
    if metrics.get("cases") != len(cases) or len(cases) < 32:
        raise ValueError("benchmark report must contain every long-horizon case")
    for name in ("mrr_at_10", "recall_at_10", "ndcg_at_10"):
        value = metrics.get(name)
        if not isinstance(value, (int, float)) or not 0.0 <= float(value) <= 1.0:
            raise ValueError(f"benchmark metric {name} must be in [0,1]")
    digest = payload.get("fixture_digest")
    if not isinstance(digest, str) or len(digest) != 64:
        raise ValueError("benchmark report fixture_digest must be a BLAKE3 hex digest")
    seen: set[str] = set()
    for case in cases:
        if not isinstance(case, dict) or not isinstance(case.get("case_id"), str):
            raise ValueError("benchmark case report is invalid")
        if case["case_id"] in seen:
            raise ValueError(f"duplicate benchmark case {case['case_id']}")
        seen.add(case["case_id"])
        if not isinstance(case.get("ordered_hits"), list):
            raise ValueError(f"{case['case_id']}: ordered_hits must be an array")
        if not isinstance(case.get("estimated_tokens"), int):
            raise ValueError(f"{case['case_id']}: estimated_tokens must be an integer")
    return payload


def threshold(value: float, minimum: float | None, name: str) -> None:
    if minimum is not None and value < minimum:
        raise ValueError(f"{name} {value:.6f} is below required {minimum:.6f}")


def run_benchmark(
    *,
    cargo: str,
    output: Path,
    release: bool,
    minimum_mrr: float | None,
    minimum_recall: float | None,
    minimum_ndcg: float | None,
) -> dict[str, Any]:
    output.parent.mkdir(parents=True, exist_ok=True)
    environment = os.environ.copy()
    environment["SUPER_MEM_LONG_HORIZON_REPORT"] = str(output)
    command = [
        cargo,
        "test",
        "-p",
        "super-mem-core",
        "--test",
        "long_horizon_fixture",
        "--locked",
    ]
    if release:
        command.append("--release")
    command.extend(["--", "--nocapture", "--test-threads=1"])
    completed = subprocess.run(command, cwd=ROOT, env=environment, check=False)
    if completed.returncode != 0:
        raise RuntimeError(f"long-horizon benchmark failed with exit {completed.returncode}")
    report = load_report(output)
    metrics = report["metrics"]
    threshold(float(metrics["mrr_at_10"]), minimum_mrr, "MRR@10")
    threshold(float(metrics["recall_at_10"]), minimum_recall, "Recall@10")
    threshold(float(metrics["ndcg_at_10"]), minimum_ndcg, "nDCG@10")
    return report


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo", default="cargo", help="cargo executable")
    parser.add_argument("--output", type=Path, help="persistent JSON report path")
    parser.add_argument("--debug", action="store_true", help="use the debug profile")
    parser.add_argument("--minimum-mrr", type=float)
    parser.add_argument("--minimum-recall", type=float)
    parser.add_argument("--minimum-ndcg", type=float)
    parser.add_argument("--json", action="store_true", help="print the report JSON")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    resolved = shutil.which(args.cargo) if os.path.basename(args.cargo) == args.cargo else args.cargo
    if not resolved:
        print(f"error: cargo executable not found: {args.cargo}", file=sys.stderr)
        return 2
    temporary: tempfile.TemporaryDirectory[str] | None = None
    try:
        if args.output:
            output = args.output.expanduser().resolve()
        else:
            temporary = tempfile.TemporaryDirectory(prefix="super-mem-long-horizon-")
            output = Path(temporary.name) / "report.json"
        report = run_benchmark(
            cargo=resolved,
            output=output,
            release=not args.debug,
            minimum_mrr=args.minimum_mrr,
            minimum_recall=args.minimum_recall,
            minimum_ndcg=args.minimum_ndcg,
        )
    except (RuntimeError, ValueError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    finally:
        if temporary is not None:
            temporary.cleanup()

    metrics = report["metrics"]
    if args.json:
        print(json.dumps(report, sort_keys=True))
    else:
        print(
            "long-horizon benchmark passed: "
            f"cases={metrics['cases']} "
            f"MRR@10={metrics['mrr_at_10']:.6f} "
            f"Recall@10={metrics['recall_at_10']:.6f} "
            f"nDCG@10={metrics['ndcg_at_10']:.6f}"
        )
        if args.output:
            print(f"report: {args.output.expanduser().resolve()}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
