from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "run_long_horizon_benchmark.py"
SPEC = importlib.util.spec_from_file_location("run_long_horizon_benchmark", SCRIPT)
assert SPEC and SPEC.loader
benchmark = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = benchmark
SPEC.loader.exec_module(benchmark)


def report_payload(cases: int = 32) -> dict:
    return {
        "schema_version": benchmark.SCHEMA,
        "fixture_digest": "a" * 64,
        "commit": None,
        "metrics": {
            "cases": cases,
            "mrr_at_10": 0.95,
            "recall_at_10": 1.0,
            "ndcg_at_10": 0.97,
        },
        "cases": [
            {
                "case_id": f"case-{index}",
                "ordered_hits": [],
                "token_budget": 768,
                "estimated_tokens": 0,
                "rendered_bytes": 0,
                "warnings": [],
            }
            for index in range(cases)
        ],
    }


class LongHorizonRunnerTests(unittest.TestCase):
    def test_load_report_validates_schema_and_case_count(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            path.write_text(json.dumps(report_payload()), encoding="utf-8")
            payload = benchmark.load_report(path)
            self.assertEqual(payload["metrics"]["cases"], 32)

    def test_threshold_failure_is_explicit(self) -> None:
        with self.assertRaisesRegex(ValueError, "below required"):
            benchmark.threshold(0.8, 0.9, "MRR@10")

    def test_run_constructs_production_test_and_reads_report(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "report.json"

            def fake_run(command, *, cwd, env, check):
                self.assertIn("long_horizon_fixture", command)
                self.assertEqual(Path(env["SUPER_MEM_LONG_HORIZON_REPORT"]), output)
                output.write_text(json.dumps(report_payload()), encoding="utf-8")
                return mock.Mock(returncode=0)

            with mock.patch.object(benchmark.subprocess, "run", side_effect=fake_run):
                payload = benchmark.run_benchmark(
                    cargo="cargo",
                    output=output,
                    release=True,
                    minimum_mrr=0.9,
                    minimum_recall=1.0,
                    minimum_ndcg=0.9,
                )
            self.assertEqual(payload["metrics"]["recall_at_10"], 1.0)


if __name__ == "__main__":
    unittest.main()
