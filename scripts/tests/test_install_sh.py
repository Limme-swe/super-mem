from __future__ import annotations

import hashlib
import os
import stat
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "install.sh"


class InstallShellTests(unittest.TestCase):
    def make_release(self, root: Path, version: str = "0.1.0", *, valid_checksum: bool = True) -> Path:
        target = "x86_64-unknown-linux-musl"
        tag_dir = root / f"v{version}"
        tag_dir.mkdir(parents=True)
        archive_root = root / f"super-mem-v{version}-{target}"
        archive_root.mkdir()
        binary = archive_root / "supermem"
        binary.write_text(f"#!/bin/sh\necho 'supermem {version}'\n", encoding="utf-8")
        binary.chmod(0o755)
        archive = tag_dir / f"super-mem-v{version}-{target}.tar.gz"
        with tarfile.open(archive, "w:gz") as bundle:
            bundle.add(archive_root, arcname=archive_root.name)
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        if not valid_checksum:
            digest = "0" * 64
        (tag_dir / "SHA256SUMS").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
        return tag_dir

    def run_installer(self, release_root: Path, install_dir: Path) -> subprocess.CompletedProcess[str]:
        environment = os.environ.copy()
        environment.update(
            {
                "HOME": str(install_dir.parent),
                "SUPER_MEM_DOWNLOAD_BASE": release_root.as_uri(),
                "SUPER_MEM_TARGET": "x86_64-unknown-linux-musl",
            }
        )
        return subprocess.run(
            ["sh", str(SCRIPT), "--version", "0.1.0", "--install-dir", str(install_dir)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=environment,
            check=False,
        )

    def test_installs_and_verifies_a_local_release(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            releases = root / "releases"
            self.make_release(releases)
            install_dir = root / "bin"
            result = self.run_installer(releases, install_dir)
            self.assertEqual(result.returncode, 0, result.stderr)
            installed = install_dir / "supermem"
            self.assertTrue(installed.is_file())
            self.assertTrue(installed.stat().st_mode & stat.S_IXUSR)
            self.assertIn("supermem 0.1.0", subprocess.check_output([installed], text=True))

    def test_refuses_a_checksum_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            releases = root / "releases"
            self.make_release(releases, valid_checksum=False)
            install_dir = root / "bin"
            result = self.run_installer(releases, install_dir)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("checksum verification failed", result.stderr)
            self.assertFalse((install_dir / "supermem").exists())

    def test_help_does_not_require_network_access(self) -> None:
        result = subprocess.run(
            ["sh", str(SCRIPT), "--help"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertEqual(result.returncode, 0)
        self.assertIn("SHA256SUMS", result.stdout)


if __name__ == "__main__":
    unittest.main()
