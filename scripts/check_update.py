#!/usr/bin/env python3
"""Check whether a newer super-mem GitHub release is available."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass

DEFAULT_API = "https://api.github.com/repos/Limme-swe/super-mem/releases/latest"
VERSION_PATTERN = re.compile(r"(?<![0-9])v?(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)")


@dataclass(frozen=True)
class Result:
    current: str
    latest: str
    update_available: bool
    release_url: str | None


def normalized(value: str) -> str:
    match = VERSION_PATTERN.search(value.strip())
    if match is None:
        raise ValueError(f"could not parse a semantic version from {value!r}")
    return match.group(1)


def version_key(value: str) -> tuple[int, int, int, int, tuple[tuple[int, int | str], ...]]:
    without_build = value.split("+", 1)[0]
    core, separator, prerelease = without_build.partition("-")
    major, minor, patch = (int(part) for part in core.split("."))
    identifiers: tuple[tuple[int, int | str], ...] = tuple(
        (0, int(part)) if part.isdigit() else (1, part)
        for part in prerelease.split(".")
        if part
    )
    # Numeric prerelease identifiers sort before text; a final release sorts last.
    return major, minor, patch, 1 if not separator else 0, identifiers


def installed_version(binary: str) -> str:
    resolved = shutil.which(binary) if os.path.basename(binary) == binary else binary
    if not resolved:
        raise RuntimeError(f"supermem executable not found: {binary}")
    try:
        completed = subprocess.run(
            [resolved, "--version"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=10,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(f"could not execute {binary}: {error}") from error
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout).strip()
        raise RuntimeError(f"{binary} --version failed: {detail or f'exit {completed.returncode}'}")
    return normalized(completed.stdout)


def latest_release(api_url: str, timeout: float) -> tuple[str, str | None]:
    request = urllib.request.Request(
        api_url,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": "super-mem-update-check",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            payload = json.load(response)
    except (OSError, urllib.error.URLError, json.JSONDecodeError) as error:
        raise RuntimeError(f"could not query the latest release: {error}") from error
    if not isinstance(payload, dict) or not isinstance(payload.get("tag_name"), str):
        raise RuntimeError("latest release response did not contain tag_name")
    latest = normalized(payload["tag_name"])
    url = payload.get("html_url")
    return latest, url if isinstance(url, str) else None


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--current", help="installed version; otherwise run supermem --version")
    parser.add_argument("--binary", default="supermem", help="supermem executable")
    parser.add_argument(
        "--api-url",
        default=os.environ.get("SUPER_MEM_RELEASE_API", DEFAULT_API),
        help="GitHub-compatible latest-release endpoint",
    )
    parser.add_argument("--timeout", type=float, default=10.0)
    parser.add_argument("--json", action="store_true", help="emit machine-readable JSON")
    parser.add_argument(
        "--require-current",
        action="store_true",
        help="exit 10 when an update is available",
    )
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.timeout <= 0:
        print("error: --timeout must be positive", file=sys.stderr)
        return 2
    try:
        current = normalized(args.current) if args.current else installed_version(args.binary)
        latest, release_url = latest_release(args.api_url, args.timeout)
    except (RuntimeError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    result = Result(
        current=current,
        latest=latest,
        update_available=version_key(latest) > version_key(current),
        release_url=release_url,
    )
    if args.json:
        print(json.dumps(asdict(result), sort_keys=True))
    elif result.update_available:
        print(f"Update available: super-mem {current} -> {latest}")
        if release_url:
            print(f"Release: {release_url}")
    else:
        print(f"super-mem {current} is current (latest: {latest})")
    return 10 if args.require_current and result.update_available else 0


if __name__ == "__main__":
    raise SystemExit(main())
