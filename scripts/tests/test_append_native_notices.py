from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import append_native_notices as notices  # noqa: E402


class AppendNativeNoticesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.root = Path(self.temporary_directory.name)
        self.report = self.root / "THIRD-PARTY-LICENSES.txt"
        self.report.write_bytes(b"cargo dependency notices\n")
        self.sysroot = self.root / "sysroot"
        self.rust_docs = self.sysroot / "share" / "doc" / "rust"
        self.rust_docs.mkdir(parents=True)
        self.apache = b"apache notice exact bytes\x00\xff"
        self.mit = b"mit notice exact bytes\x00\xfe"
        (self.rust_docs / "LICENSE-APACHE").write_bytes(self.apache)
        (self.rust_docs / "LICENSE-MIT").write_bytes(self.mit)

    def test_prefers_library_notice_and_preserves_exact_bytes(self) -> None:
        library = b"library notice exact bytes\x00\xfd"
        full = b"full toolchain notice must not be selected"
        (self.rust_docs / "COPYRIGHT-library.html").write_bytes(library)
        (self.rust_docs / "COPYRIGHT.html").write_bytes(full)

        notices.append_rust_notices(self.report, self.sysroot)

        output = self.report.read_bytes()
        self.assertIn(library, output)
        self.assertNotIn(full, output)
        self.assertIn(self.apache, output)
        self.assertIn(self.mit, output)
        self.assertIn(b"COPYRIGHT-library.html", output)

    def test_falls_back_to_full_toolchain_notice(self) -> None:
        full = b"full toolchain fallback exact bytes\x00\xfc"
        (self.rust_docs / "COPYRIGHT.html").write_bytes(full)

        notices.append_rust_notices(self.report, self.sysroot)

        output = self.report.read_bytes()
        self.assertIn(full, output)
        self.assertIn(b"COPYRIGHT.html", output)

    def test_missing_copyright_notice_fails_closed(self) -> None:
        with self.assertRaisesRegex(SystemExit, "required native license notice is missing"):
            notices.append_rust_notices(self.report, self.sysroot)

    def test_missing_either_toolchain_license_fails_closed(self) -> None:
        (self.rust_docs / "COPYRIGHT-library.html").write_bytes(b"library notice")
        for missing in ("LICENSE-APACHE", "LICENSE-MIT"):
            with self.subTest(missing=missing):
                path = self.rust_docs / missing
                content = path.read_bytes()
                path.unlink()
                try:
                    with self.assertRaisesRegex(SystemExit, str(path)):
                        notices.append_rust_notices(self.report, self.sysroot)
                finally:
                    path.write_bytes(content)


if __name__ == "__main__":
    unittest.main()
