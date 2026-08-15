from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "uninstall.sh"


class UninstallShellTests(unittest.TestCase):
    def run_script(self, home: Path, *args: str) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        environment.update({"HOME": str(home), "XDG_DATA_HOME": str(home / "data")})
        return subprocess.run(
            ["sh", str(SCRIPT), *args],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment,
            check=False,
        )

    def test_removes_binary_but_preserves_memories_by_default(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            install = home / "bin"
            install.mkdir()
            binary = install / "supermem"
            binary.write_text("binary", encoding="utf-8")
            data = home / "data" / "super-mem"
            data.mkdir(parents=True)
            (data / "memory.sqlite3").write_text("memory", encoding="utf-8")
            result = self.run_script(home, "--install-dir", str(install))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(binary.exists())
            self.assertTrue(data.exists())
            self.assertIn("preserved", result.stdout)

    def test_purge_requires_explicit_confirmation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            result = self.run_script(home, "--purge-data")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("requires --yes", result.stderr)

    def test_confirmed_purge_deletes_only_default_data_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            data = home / "data" / "super-mem"
            data.mkdir(parents=True)
            outside = home / "keep.txt"
            outside.write_text("keep", encoding="utf-8")
            result = self.run_script(home, "--purge-data", "--yes")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(data.exists())
            self.assertTrue(outside.exists())

    def test_dry_run_changes_nothing(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            install = home / "bin"
            install.mkdir()
            binary = install / "supermem"
            binary.write_text("binary", encoding="utf-8")
            result = self.run_script(home, "--install-dir", str(install), "--dry-run")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(binary.exists())
            self.assertIn("Would remove", result.stdout)


if __name__ == "__main__":
    unittest.main()
