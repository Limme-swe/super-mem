#!/usr/bin/env python3
"""Create a bounded, path-sanitized support report without copying memory contents."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import sys
import tempfile
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

MAX_STREAM = 64 * 1024
MAX_INPUT = 1024 * 1024
PATH_KEYS = {
    "path",
    "cwd",
    "root",
    "directory",
    "database",
    "database_path",
    "db",
    "binary",
    "executable",
    "repository_path",
    "git_dir",
    "common_dir",
    "temp",
}
CREDENTIAL_URL = re.compile(r"(https?://)([^/@\s:]+):([^/@\s]+)@", re.IGNORECASE)


@dataclass(frozen=True)
class CommandReport:
    name: str
    exit_code: int | None
    timed_out: bool
    stdout: Any
    stderr: str


def digest_label(value: str) -> str:
    digest = hashlib.sha256(value.encode("utf-8", errors="replace")).hexdigest()[:12]
    return f"<redacted-path:{digest}>"


def known_replacements(cwd: Path) -> list[tuple[str, str]]:
    values = {
        str(Path.home().resolve()): "<HOME>",
        str(cwd.resolve(strict=False)): "<CWD>",
        str(Path(tempfile.gettempdir()).resolve()): "<TEMP>",
    }
    # Windows tools may use either separator even when Python normalized one form.
    expanded = dict(values)
    for value, replacement in list(values.items()):
        expanded[value.replace("/", "\\")] = replacement
        expanded[value.replace("\\", "/")] = replacement
    return sorted(expanded.items(), key=lambda item: len(item[0]), reverse=True)


def sanitize_string(value: str, replacements: list[tuple[str, str]], *, path_context: bool) -> str:
    sanitized = CREDENTIAL_URL.sub(r"\1<credentials-redacted>@", value)
    for needle, replacement in replacements:
        if needle:
            sanitized = sanitized.replace(needle, replacement)
    if path_context and sanitized == value and value:
        return digest_label(value)
    return sanitized[:MAX_STREAM] + ("\n<truncated>" if len(sanitized) > MAX_STREAM else "")


def sanitize(value: Any, replacements: list[tuple[str, str]], *, key: str = "") -> Any:
    lowered = key.lower()
    path_context = lowered in PATH_KEYS or lowered.endswith(("_path", "_dir", "_directory", "_root"))
    if isinstance(value, str):
        return sanitize_string(value, replacements, path_context=path_context)
    if isinstance(value, list):
        return [sanitize(item, replacements, key=key) for item in value[:500]]
    if isinstance(value, dict):
        return {
            str(item_key): sanitize(item_value, replacements, key=str(item_key))
            for item_key, item_value in list(value.items())[:500]
        }
    return value


def parse_output(text: str, replacements: list[tuple[str, str]]) -> Any:
    bounded_input = text[:MAX_INPUT]
    if not bounded_input.strip():
        return None
    try:
        return sanitize(json.loads(bounded_input), replacements)
    except json.JSONDecodeError:
        plain = text[:MAX_STREAM]
        if len(text) > MAX_STREAM:
            plain += "\n<truncated>"
        return sanitize_string(plain, replacements, path_context=False)


def resolve_binary(binary: str) -> str:
    if os.path.dirname(binary):
        path = Path(binary).expanduser()
        if path.is_file():
            return str(path.resolve())
    else:
        resolved = shutil.which(binary)
        if resolved:
            return resolved
    raise RuntimeError(f"supermem executable not found: {binary}")


def run_report(name: str, command: list[str], *, cwd: Path, timeout: float, replacements: list[tuple[str, str]]) -> CommandReport:
    environment = os.environ.copy()
    environment.update({"GIT_TERMINAL_PROMPT": "0", "GIT_OPTIONAL_LOCKS": "0"})
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            env=environment,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
        return CommandReport(
            name=name,
            exit_code=result.returncode,
            timed_out=False,
            stdout=parse_output(result.stdout, replacements),
            stderr=sanitize_string(result.stderr, replacements, path_context=False),
        )
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout.decode(errors="replace") if isinstance(error.stdout, bytes) else (error.stdout or "")
        stderr = error.stderr.decode(errors="replace") if isinstance(error.stderr, bytes) else (error.stderr or "")
        return CommandReport(
            name=name,
            exit_code=None,
            timed_out=True,
            stdout=parse_output(stdout, replacements),
            stderr=sanitize_string(stderr, replacements, path_context=False),
        )
    except OSError as error:
        return CommandReport(
            name=name,
            exit_code=None,
            timed_out=False,
            stdout=None,
            stderr=sanitize_string(str(error), replacements, path_context=False),
        )


def write_private(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    encoded = (json.dumps(payload, indent=2, sort_keys=True) + "\n").encode("utf-8")
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(encoded)
    except Exception:
        try:
            os.close(descriptor)
        except OSError:
            pass
        raise


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="supermem", help="supermem executable")
    parser.add_argument("--cwd", type=Path, default=Path.cwd(), help="repository to diagnose")
    parser.add_argument("--output", type=Path, help="destination JSON file")
    parser.add_argument("--timeout", type=float, default=20.0, help="per-command timeout")
    parser.add_argument("--skip-doctor", action="store_true", help="collect only version/platform metadata")
    parser.add_argument("--strict", action="store_true", help="exit nonzero when a collected command fails")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.timeout <= 0:
        print("error: --timeout must be positive", file=sys.stderr)
        return 2
    cwd = args.cwd.expanduser().resolve(strict=False)
    if not cwd.is_dir():
        print(f"error: working directory is not a directory: {cwd}", file=sys.stderr)
        return 2
    try:
        binary = resolve_binary(args.binary)
    except RuntimeError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    replacements = known_replacements(cwd)
    reports = [run_report("version", [binary, "--version"], cwd=cwd, timeout=args.timeout, replacements=replacements)]
    if not args.skip_doctor:
        reports.append(
            run_report(
                "doctor",
                [binary, "--json", "doctor", "--cwd", str(cwd)],
                cwd=cwd,
                timeout=args.timeout,
                replacements=replacements,
            )
        )

    generated = datetime.now(timezone.utc).replace(microsecond=0)
    output = args.output or Path.cwd() / f"super-mem-support-{generated.strftime('%Y%m%dT%H%M%SZ')}.json"
    payload = {
        "format": "super-mem-support-v1",
        "generated_at": generated.isoformat(),
        "privacy_notice": (
            "Memory rows, database files, environment variables, and Git remotes are not copied. "
            "Known local paths are replaced or hashed. Review this report before sharing it."
        ),
        "platform": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "python": platform.python_version(),
        },
        "commands": [asdict(report) for report in reports],
    }
    try:
        write_private(output, payload)
    except OSError as error:
        print(f"error: could not write support report: {error}", file=sys.stderr)
        return 2
    print(output)
    failed = any(report.timed_out or report.exit_code not in (0,) for report in reports)
    return 1 if args.strict and failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
