from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "check_docs.py"
SPEC = importlib.util.spec_from_file_location("check_docs", SCRIPT)
assert SPEC and SPEC.loader
check_docs = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = check_docs
SPEC.loader.exec_module(check_docs)


class CheckDocsTests(unittest.TestCase):
    def test_accepts_existing_relative_and_repository_root_links(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "docs").mkdir()
            (root / "README.md").write_text("[guide](docs/guide.md)\n", encoding="utf-8")
            (root / "docs" / "guide.md").write_text("[home](/README.md#top)\n", encoding="utf-8")
            self.assertEqual(check_docs.main(["--root", str(root)]), 0)

    def test_reports_missing_links_but_ignores_fenced_examples(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text(
                "[missing](docs/nope.md)\n```md\n[example](also-missing.md)\n```\n",
                encoding="utf-8",
            )
            self.assertEqual(check_docs.main(["--root", str(root)]), 1)

    def test_rejects_links_that_escape_the_repository(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "README.md").write_text("[outside](../secret.md)\n", encoding="utf-8")
            problems = check_docs.validate_file(root, root / "README.md")
            self.assertEqual(len(problems), 1)
            self.assertIn("escapes", problems[0].reason)


if __name__ == "__main__":
    unittest.main()
