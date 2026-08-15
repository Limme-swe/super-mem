from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "generate_long_horizon_fixture.py"
SPEC = importlib.util.spec_from_file_location("generate_long_horizon_fixture", SCRIPT)
assert SPEC and SPEC.loader
generator = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = generator
SPEC.loader.exec_module(generator)


class LongHorizonGeneratorTests(unittest.TestCase):
    def test_generator_is_deterministic_and_complete(self) -> None:
        first_cases, first_qrels = generator.generate()
        second_cases, second_qrels = generator.generate()
        self.assertEqual(first_cases, second_cases)
        self.assertEqual(first_qrels, second_qrels)
        self.assertEqual(len(first_cases), 32)
        self.assertEqual({case["case_id"] for case in first_cases}, {row["case_id"] for row in first_qrels})
        self.assertGreaterEqual(sum(len(case["operations"]) for case in first_cases), 300)

    def test_write_then_check_round_trip(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            self.assertEqual(generator.write_or_check(output, False), 0)
            self.assertEqual(generator.write_or_check(output, True), 0)
            cases = [json.loads(line) for line in (output / "v1.jsonl").read_text().splitlines()]
            self.assertEqual(len(cases), 32)


if __name__ == "__main__":
    unittest.main()
