from __future__ import annotations

import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "preflight.py"
SPEC = importlib.util.spec_from_file_location("preflight", SCRIPT)
assert SPEC and SPEC.loader
preflight = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = preflight
SPEC.loader.exec_module(preflight)


class PreflightTests(unittest.TestCase):
    def test_default_database_honors_explicit_override(self) -> None:
        with mock.patch.dict(os.environ, {"SUPER_MEM_DB": "~/custom/memory.sqlite3"}, clear=False):
            self.assertEqual(preflight.default_database(), Path("~/custom/memory.sqlite3").expanduser())

    def test_missing_binary_is_a_failure(self) -> None:
        with mock.patch.object(preflight, "resolve_program", return_value=None):
            check, resolved = preflight.check_binary("supermem")
        self.assertEqual(check.status, "fail")
        self.assertIsNone(resolved)

    def test_writable_missing_database_path_passes_without_creating_it(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "nested" / "memory.sqlite3"
            check = preflight.check_database_path(database)
            self.assertEqual(check.status, "pass")
            self.assertFalse(database.exists())
            self.assertFalse(database.parent.exists())

    def test_directory_cannot_be_used_as_database(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            check = preflight.check_database_path(Path(directory))
            self.assertEqual(check.status, "fail")

    def test_main_reports_failure_for_missing_working_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            missing = Path(directory) / "missing"
            with mock.patch.object(preflight, "check_binary", return_value=(preflight.Check("supermem", "pass", "ok"), "/bin/true")):
                result = preflight.main(["--cwd", str(missing), "--db", str(Path(directory) / "memory.sqlite3"), "--json"])
            self.assertEqual(result, 1)


if __name__ == "__main__":
    unittest.main()
