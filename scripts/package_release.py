#!/usr/bin/env python3
"""Create a deterministic Super Mem release archive."""

from __future__ import annotations

import argparse
import gzip
import io
import os
import re
import subprocess
import tarfile
import time
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SAFE_COMPONENT = re.compile(r"[0-9A-Za-z_.+-]+")


def epoch() -> int:
    value = os.environ.get("SOURCE_DATE_EPOCH", "315532800")
    try:
        parsed = int(value)
    except ValueError as error:
        raise SystemExit("SOURCE_DATE_EPOCH must be an integer") from error
    return max(parsed, 0)


def tracked_entries(archive_root: str) -> list[tuple[str, bytes, int]]:
    result = subprocess.run(
        ["git", "ls-files", "--stage", "-z"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise SystemExit(
            "cannot enumerate the release tree with git:\n"
            + result.stderr.decode("utf-8", errors="replace")
        )

    packaged: list[tuple[str, bytes, int]] = []
    for record in result.stdout.split(b"\0"):
        if not record:
            continue
        metadata, raw_name = record.split(b"\t", 1)
        git_mode, blob_sha, stage = metadata.split(b" ")
        if git_mode not in {b"100644", b"100755"}:
            raise SystemExit(
                f"unsupported tracked entry mode {git_mode.decode()}: "
                f"{raw_name.decode('utf-8', errors='replace')}"
            )
        if stage != b"0":
            raise SystemExit(
                f"release index contains an unresolved stage for "
                f"{raw_name.decode('utf-8', errors='replace')}"
            )
        name = raw_name.decode("utf-8")
        blob = subprocess.run(
            ["git", "cat-file", "blob", blob_sha.decode("ascii")],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if blob.returncode != 0:
            raise SystemExit(
                f"cannot read indexed Git blob for {name}:\n"
                + blob.stderr.decode("utf-8", errors="replace")
            )
        mode = 0o755 if git_mode == b"100755" else 0o644
        packaged.append((f"{archive_root}/{name}", blob.stdout, mode))
    return packaged


def entries(binary: Path, third_party_licenses: Path, archive_root: str) -> list[tuple[str, bytes, int]]:
    binary_name = "supermem.exe" if binary.suffix.lower() == ".exe" else "supermem"
    packaged = [(f"{archive_root}/{binary_name}", binary.read_bytes(), 0o755)]
    packaged.extend(tracked_entries(archive_root))
    if not third_party_licenses.is_file():
        raise SystemExit(f"third-party license report is missing: {third_party_licenses}")
    packaged.append(
        (
            f"{archive_root}/THIRD-PARTY-LICENSES.txt",
            third_party_licenses.read_bytes(),
            0o644,
        )
    )
    names = [name for name, _, _ in packaged]
    if len(names) != len(set(names)):
        raise SystemExit("release archive would contain duplicate paths")
    return packaged


def write_tar(path: Path, archive_root: str, packaged: list[tuple[str, bytes, int]], timestamp: int) -> None:
    with path.open("wb") as raw:
        with gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=timestamp, compresslevel=9) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                directory = tarfile.TarInfo(archive_root)
                directory.type = tarfile.DIRTYPE
                directory.mode = 0o755
                directory.mtime = timestamp
                archive.addfile(directory)
                for name, content, mode in packaged:
                    item = tarfile.TarInfo(name)
                    item.size = len(content)
                    item.mode = mode
                    item.mtime = timestamp
                    archive.addfile(item, io.BytesIO(content))


def write_zip(path: Path, archive_root: str, packaged: list[tuple[str, bytes, int]], timestamp: int) -> None:
    clamped = min(max(timestamp, 315532800), 4354819198)
    date_time = time.gmtime(clamped)[:6]
    with zipfile.ZipFile(path, mode="w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        directory = zipfile.ZipInfo(f"{archive_root}/", date_time=date_time)
        directory.create_system = 3
        directory.external_attr = (0o40755 << 16) | 0x10
        archive.writestr(directory, b"")
        for name, content, mode in packaged:
            item = zipfile.ZipInfo(name, date_time=date_time)
            item.create_system = 3
            item.compress_type = zipfile.ZIP_DEFLATED
            item.external_attr = mode << 16
            archive.writestr(item, content, compress_type=zipfile.ZIP_DEFLATED, compresslevel=9)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--third-party-licenses", type=Path, required=True)
    arguments = parser.parse_args()

    if not arguments.binary.is_file():
        raise SystemExit(f"release binary does not exist: {arguments.binary}")
    for label, value in (("target", arguments.target), ("version", arguments.version)):
        if SAFE_COMPONENT.fullmatch(value) is None:
            raise SystemExit(f"unsafe {label}: {value!r}")

    archive_root = f"super-mem-v{arguments.version}-{arguments.target}"
    extension = ".zip" if "windows" in arguments.target else ".tar.gz"
    arguments.output_dir.mkdir(parents=True, exist_ok=True)
    destination = arguments.output_dir / f"{archive_root}{extension}"
    packaged = entries(arguments.binary, arguments.third_party_licenses, archive_root)
    timestamp = epoch()
    if extension == ".zip":
        write_zip(destination, archive_root, packaged, timestamp)
    else:
        write_tar(destination, archive_root, packaged, timestamp)
    print(destination)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
