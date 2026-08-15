from __future__ import annotations

import importlib.util
import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "support_bundle.py"
SPEC = importlib.util.spec_from_file_location("support_bundle", SCRIPT)
assert SPEC and SPEC.loader
support_bundle = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = support_bundle
SPEC.loader.exec_module(support_bundle)


class SupportBundleTests(unittest.TestCase):
    def make_binary(self, root: Path) -> Path:
        binary = root / "supermem"
        binary.write_text(
            """#!/bin/sh
if [ "$1" = "--version" ]; then
  echo 'supermem 0.1.0'
  exit 0
fi
printf '{"database_path":"%s/private/memory.sqlite3","repository":{"root":"%s/project"}}\\n' "$HOME" "$HOME"
""",
            encoding="utf-8",
        )
        binary.chmod(0o755)
        return binary

    def test_report_redacts_home_paths_and_uses_private_permissions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = self.make_binary(root)
            output = root / "bundle.json"
            result = support_bundle.main(
                ["--binary", str(binary), "--cwd", str(root), "--output", str(output)]
            )
            self.assertEqual(result, 0)
            raw = output.read_text(encoding="utf-8")
            self.assertNotIn(str(Path.home()), raw)
            payload = json.loads(raw)
            self.assertEqual(payload["format"], "super-mem-support-v1")
            doctor = next(item for item in payload["commands"] if item["name"] == "doctor")
            self.assertTrue(str(doctor["stdout"]["database_path"]).startswith(("<HOME>", "<redacted-path:")))
            if os.name != "nt":
                self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)

    def test_strict_mode_propagates_command_failure_after_writing_report(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "supermem"
            binary.write_text("#!/bin/sh\necho failure >&2\nexit 9\n", encoding="utf-8")
            binary.chmod(0o755)
            output = root / "bundle.json"
            result = support_bundle.main(
                [
                    "--binary",
                    str(binary),
                    "--cwd",
                    str(root),
                    "--output",
                    str(output),
                    "--skip-doctor",
                    "--strict",
                ]
            )
            self.assertEqual(result, 1)
            self.assertTrue(output.exists())

    def test_url_credentials_are_removed(self) -> None:
        sanitized = support_bundle.sanitize_string(
            "https://alice:secret@example.invalid/repo", [], path_context=False
        )
        self.assertNotIn("secret", sanitized)
        self.assertIn("credentials-redacted", sanitized)


if __name__ == "__main__":
    unittest.main()
