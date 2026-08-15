from __future__ import annotations

import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "smoke_install.py"
SPEC = importlib.util.spec_from_file_location("smoke_install", SCRIPT)
assert SPEC and SPEC.loader
smoke_install = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = smoke_install
SPEC.loader.exec_module(smoke_install)


class SmokeInstallTests(unittest.TestCase):
    def make_binary(self, root: Path) -> tuple[Path, Path]:
        log = root / "commands.log"
        binary = root / "supermem"
        binary.write_text(
            """#!/bin/sh
printf '%s\\n' "$*" >> "$FAKE_LOG"
previous=''
for argument in "$@"; do
  if [ "$previous" = '--output' ]; then
    printf '{"snapshot":true}\\n' > "$argument"
  fi
  previous=$argument
done
if [ "$1" = '--version' ]; then echo 'supermem 0.1.0'; fi
exit 0
""",
            encoding="utf-8",
        )
        binary.chmod(0o755)
        return binary, log

    def test_complete_workflow_uses_two_temporary_databases(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary, log = self.make_binary(root)
            work = root / "work"
            with mock.patch.dict(os.environ, {"FAKE_LOG": str(log), "SUPER_MEM_DB": "/must/not/be/used"}, clear=False):
                result = smoke_install.main(
                    ["--binary", str(binary), "--work-dir", str(work), "--json"]
                )
            self.assertEqual(result, 0)
            commands = log.read_text(encoding="utf-8")
            self.assertIn(str(work / "primary.sqlite3"), commands)
            self.assertIn(str(work / "restored.sqlite3"), commands)
            self.assertNotIn("/must/not/be/used", commands)
            self.assertTrue((work / "memory.jsonl").is_file())

    def test_stops_after_a_failed_step(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "supermem"
            binary.write_text(
                "#!/bin/sh\ncase \"$*\" in *remember*) exit 5;; esac\nexit 0\n",
                encoding="utf-8",
            )
            binary.chmod(0o755)
            work = root / "work"
            result = smoke_install.main(["--binary", str(binary), "--work-dir", str(work)])
            self.assertEqual(result, 1)

    def test_refuses_nonempty_work_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary, _ = self.make_binary(root)
            work = root / "work"
            work.mkdir()
            (work / "keep").write_text("data", encoding="utf-8")
            with mock.patch.dict(os.environ, {"FAKE_LOG": str(root / "log")}, clear=False):
                result = smoke_install.main(["--binary", str(binary), "--work-dir", str(work)])
            self.assertEqual(result, 2)
            self.assertTrue((work / "keep").exists())


if __name__ == "__main__":
    unittest.main()
