#!/usr/bin/env python3
"""Smoke-test a built binary or an extracted release archive."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def run(binary: Path, arguments: list[str], *, stdin: str | None = None) -> str:
    environment = os.environ.copy()
    path_key = next(
        (key for key in environment if key.upper() == "PATH"),
        "PATH",
    )
    existing_path = environment.get(path_key)
    environment[path_key] = os.pathsep.join(
        part for part in (str(binary.parent), existing_path) if part
    )
    result = subprocess.run(
        [str(binary), *arguments],
        env=environment,
        input=stdin,
        text=True,
        encoding="utf-8",
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
        timeout=30,
    )
    if result.returncode != 0:
        raise SystemExit(
            f"command failed ({result.returncode}): {binary} {' '.join(arguments)}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return result.stdout


def safe_destination(root: Path, member: str) -> Path:
    destination = (root / member).resolve()
    if destination != root and root not in destination.parents:
        raise SystemExit(f"archive path escapes its root: {member}")
    return destination


def extract(archive: Path, destination: Path) -> Path:
    if archive.name.endswith(".tar.gz"):
        with tarfile.open(archive, "r:gz") as source:
            for member in source.getmembers():
                safe_destination(destination, member.name)
                if member.issym() or member.islnk():
                    raise SystemExit(f"release archive contains a link: {member.name}")
            source.extractall(destination, filter="data")
    elif archive.suffix.lower() == ".zip":
        with zipfile.ZipFile(archive) as source:
            for member in source.infolist():
                safe_destination(destination, member.filename)
                # A Unix-created ZIP can encode a symbolic link in the upper
                # mode bits even though ZipInfo has no first-class link type.
                mode = member.external_attr >> 16
                if member.create_system == 3 and mode & 0o170000 == 0o120000:
                    raise SystemExit(
                        f"release archive contains a link: {member.filename}"
                    )
            source.extractall(destination)
    else:
        raise SystemExit(f"unsupported release archive: {archive}")

    names = {"supermem", "supermem.exe"}
    binaries = [path for path in destination.rglob("*") if path.is_file() and path.name in names]
    if len(binaries) != 1:
        raise SystemExit(f"expected one packaged binary, found {len(binaries)}")
    binary = binaries[0]
    if binary.name == "supermem":
        binary.chmod(binary.stat().st_mode | 0o111)
    return binary


def exercise(binary: Path, version: str) -> None:
    with tempfile.TemporaryDirectory(prefix="super mem release smoke å ") as directory:
        root = Path(directory)
        first = root / "first memory.sqlite3"
        second = root / "restored memory.sqlite3"
        snapshot = root / "memory snapshot.jsonl"
        sentinel = "packaged release sentinel copper-lark"

        version_output = run(binary, ["--version"]).strip()
        if version_output != f"supermem {version}":
            raise SystemExit(f"unexpected version output: {version_output!r}")
        run(binary, ["--db", str(first), "init"])
        run(
            binary,
            ["--db", str(first), "remember", "--kind", "decision", "--body-stdin"],
            stdin=sentinel,
        )
        recalled = run(
            binary,
            ["--db", str(first), "recall", "--query-stdin", "--format", "context"],
            stdin="packaged release sentinel",
        )
        if sentinel not in recalled:
            raise SystemExit("fresh store did not recall the release sentinel")
        run(binary, ["--db", str(first), "export", "--output", str(snapshot)])
        run(binary, ["--db", str(second), "import", str(snapshot)])
        restored = run(
            binary,
            ["--db", str(second), "recall", "--query", "copper-lark", "--format", "context"],
        )
        if sentinel not in restored:
            raise SystemExit("restored store did not recall the release sentinel")
        run(binary, ["--db", str(second), "doctor"])

        # Exercise the actual stdio MCP transport from the packaged binary.
        # The record and server share an explicit root/namespace so this also
        # checks scope propagation rather than merely listing tool metadata.
        mcp_namespace = "release-smoke"
        run(
            binary,
            [
                "--db",
                str(first),
                "remember",
                "--namespace",
                mcp_namespace,
                "--cwd",
                str(root),
                "--body",
                sentinel,
            ],
        )
        protocol = "\n".join(
            (
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "initialize",
                        "params": {
                            "protocolVersion": "2025-11-25",
                            "capabilities": {},
                            "clientInfo": {
                                "name": "super-mem-release-smoke",
                                "version": version,
                            },
                        },
                    },
                    separators=(",", ":"),
                ),
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "method": "notifications/initialized",
                    },
                    separators=(",", ":"),
                ),
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "tools/call",
                        "params": {
                            "name": "memory_context",
                            "arguments": {"query": "copper-lark"},
                        },
                    },
                    separators=(",", ":"),
                ),
                "",
            )
        )
        mcp_output = run(
            binary,
            [
                "--db",
                str(first),
                "mcp",
                "--root",
                str(root),
                "--namespace",
                mcp_namespace,
            ],
            stdin=protocol,
        )
        messages = [json.loads(line) for line in mcp_output.splitlines()]
        response = next(
            (message for message in messages if message.get("id") == 2), None
        )
        if response is None or response.get("result", {}).get("isError") is True:
            raise SystemExit("packaged MCP server did not return a successful context result")
        content = response.get("result", {}).get("content", [])
        rendered = "\n".join(
            item.get("text", "") for item in content if isinstance(item, dict)
        )
        if sentinel not in rendered:
            raise SystemExit("packaged MCP server did not recall the release sentinel")


def main() -> int:
    parser = argparse.ArgumentParser()
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--binary", type=Path)
    source.add_argument("--archive", type=Path)
    parser.add_argument("--version")
    arguments = parser.parse_args()
    version = arguments.version
    if version is None:
        with (ROOT / ".github/release.toml").open("rb") as source_file:
            version = tomllib.load(source_file)["version"]

    if arguments.binary:
        exercise(arguments.binary.resolve(), version)
    else:
        with tempfile.TemporaryDirectory(prefix="super-mem-extract-") as directory:
            binary = extract(arguments.archive.resolve(), Path(directory).resolve())
            exercise(binary, version)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
