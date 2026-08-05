#!/usr/bin/env python3
"""Verify the complete release set and write SHA256SUMS."""

from __future__ import annotations

import argparse
import hashlib
import subprocess
import tarfile
import zipfile
from pathlib import Path


TARGETS = (
    ("x86_64-unknown-linux-musl", ".tar.gz"),
    ("x86_64-pc-windows-msvc", ".zip"),
    ("x86_64-apple-darwin", ".tar.gz"),
    ("aarch64-apple-darwin", ".tar.gz"),
)


def tracked_files() -> dict[str, str]:
    root = Path(__file__).resolve().parents[1]
    result = subprocess.run(
        ["git", "ls-files", "--stage", "-z"],
        cwd=root,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise SystemExit(
            "cannot enumerate the release tree with git:\n"
            + result.stderr.decode("utf-8", errors="replace")
        )
    files: dict[str, str] = {}
    for record in result.stdout.split(b"\0"):
        if not record:
            continue
        metadata, raw_name = record.split(b"\t", 1)
        _mode, blob_sha, stage = metadata.split(b" ")
        if stage != b"0":
            raise SystemExit(
                f"release index contains an unresolved stage for "
                f"{raw_name.decode('utf-8', errors='replace')}"
            )
        name = raw_name.decode("utf-8")
        blob = subprocess.run(
            ["git", "cat-file", "blob", blob_sha.decode("ascii")],
            cwd=root,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if blob.returncode != 0:
            raise SystemExit(
                f"cannot read indexed Git blob for {name}:\n"
                + blob.stderr.decode("utf-8", errors="replace")
            )
        files[name] = hashlib.sha256(blob.stdout).hexdigest()
    return files


def validate_member_name(name: str, root: str) -> None:
    normalized = name[:-1] if name.endswith("/") else name
    parts = normalized.split("/")
    if (
        not normalized
        or "\\" in normalized
        or any(part in {"", ".", ".."} for part in parts)
        or parts[0] != root
    ):
        raise SystemExit(f"unsafe release archive member: {name!r}")


def member_digests(path: Path, root: str) -> dict[str, str]:
    digests: dict[str, str] = {}
    if path.name.endswith(".tar.gz"):
        with tarfile.open(path, "r:gz") as archive:
            seen: set[str] = set()
            for member in archive.getmembers():
                validate_member_name(member.name, root)
                if member.name in seen:
                    raise SystemExit(f"duplicate release archive member: {member.name}")
                seen.add(member.name)
                if member.isdir():
                    if member.name.rstrip("/") != root:
                        raise SystemExit(
                            f"unexpected directory member in release archive: {member.name}"
                        )
                    continue
                if not member.isfile():
                    raise SystemExit(
                        f"release archive contains a link or special member: {member.name}"
                    )
                source = archive.extractfile(member)
                if source is None:
                    raise SystemExit(f"cannot read release archive member: {member.name}")
                hasher = hashlib.sha256()
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    hasher.update(chunk)
                digests[member.name] = hasher.hexdigest()
            return digests
    with zipfile.ZipFile(path) as archive:
        seen = set()
        for member in archive.infolist():
            validate_member_name(member.filename, root)
            if member.filename in seen:
                raise SystemExit(f"duplicate release archive member: {member.filename}")
            seen.add(member.filename)
            if member.flag_bits & 0x1:
                raise SystemExit(f"encrypted release archive member: {member.filename}")
            mode = member.external_attr >> 16
            if member.create_system == 3 and mode & 0o170000 == 0o120000:
                raise SystemExit(f"release archive contains a link: {member.filename}")
            if member.is_dir():
                if member.filename.rstrip("/") != root:
                    raise SystemExit(
                        f"unexpected directory member in release archive: {member.filename}"
                    )
                continue
            with archive.open(member) as source:
                hasher = hashlib.sha256()
                for chunk in iter(lambda: source.read(1024 * 1024), b""):
                    hasher.update(chunk)
            digests[member.filename] = hasher.hexdigest()
        return digests


def digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--version", required=True)
    arguments = parser.parse_args()

    expected = {
        f"super-mem-v{arguments.version}-{target}{extension}" for target, extension in TARGETS
    }
    actual = {
        path.name
        for path in arguments.directory.iterdir()
        if path.is_file() and (path.name.endswith(".tar.gz") or path.suffix == ".zip")
    }
    if actual != expected:
        raise SystemExit(
            f"release asset set mismatch; missing={sorted(expected - actual)}, "
            f"unexpected={sorted(actual - expected)}"
        )

    source_files = tracked_files()
    lines = []
    for name in sorted(expected):
        path = arguments.directory / name
        target = next(target for target, _ in TARGETS if target in name)
        root = f"super-mem-v{arguments.version}-{target}"
        binary = "supermem.exe" if "windows" in target else "supermem"
        expected_members = {
            f"{root}/{binary}",
            f"{root}/THIRD-PARTY-LICENSES.txt",
            *(f"{root}/{item}" for item in source_files),
        }
        archive_digests = member_digests(path, root)
        actual_members = set(archive_digests)
        if actual_members != expected_members:
            raise SystemExit(
                f"{name} contents mismatch; missing={sorted(expected_members - actual_members)}, "
                f"unexpected={sorted(actual_members - expected_members)}"
            )
        for source_name, expected_digest in source_files.items():
            member = f"{root}/{source_name}"
            if archive_digests[member] != expected_digest:
                raise SystemExit(
                    f"{name} contains bytes for {source_name} that differ from the release Git tree"
                )
        if path.stat().st_size < 100_000:
            raise SystemExit(f"release archive is implausibly small: {name}")
        lines.append(f"{digest(path)}  {name}")

    checksum = arguments.directory / "SHA256SUMS"
    checksum.write_text("\n".join(lines) + "\n", encoding="ascii", newline="\n")
    print(checksum)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
