from __future__ import annotations

import importlib.util
import io
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "dev.py"
SPEC = importlib.util.spec_from_file_location("dev", SCRIPT)
assert SPEC and SPEC.loader
dev = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = dev
SPEC.loader.exec_module(dev)


class DevRunnerTests(unittest.TestCase):
    def test_dry_run_prints_without_spawning_processes(self) -> None:
        output = io.StringIO()
        with mock.patch.object(dev.subprocess, "run") as run, redirect_stdout(output):
            result = dev.main(["quick", "--dry-run"])
        self.assertEqual(result, 0)
        run.assert_not_called()
        self.assertIn("cargo check", output.getvalue())
        self.assertIn("check_docs.py", output.getvalue())

    def test_stops_on_first_failure_by_default(self) -> None:
        completed = [mock.Mock(returncode=0), mock.Mock(returncode=7)]
        with mock.patch.object(dev.subprocess, "run", side_effect=completed) as run:
            result = dev.run_steps(dev.TARGETS["rust"], dry_run=False, keep_going=False)
        self.assertEqual(result, 1)
        self.assertEqual(run.call_count, 2)

    def test_keep_going_runs_every_step(self) -> None:
        with mock.patch.object(dev.subprocess, "run", return_value=mock.Mock(returncode=3)) as run:
            result = dev.run_steps(dev.TARGETS["fmt"], dry_run=False, keep_going=True)
        self.assertEqual(result, 1)
        self.assertEqual(run.call_count, 1)

    def test_full_target_contains_release_relevant_checks(self) -> None:
        commands = [step.command for step in dev.TARGETS["full"]]
        flattened = [" ".join(command) for command in commands]
        self.assertTrue(any("cargo clippy" in command for command in flattened))
        self.assertTrue(any("cargo test" in command for command in flattened))
        self.assertTrue(any("cargo package" in command for command in flattened))
        self.assertTrue(any("unittest discover" in command for command in flattened))


if __name__ == "__main__":
    unittest.main()
