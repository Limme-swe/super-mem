#!/usr/bin/env python3
"""Run non-destructive checks before first use or adapter installation."""

from __future__ import annotations

import argparse
import json
import os
import platform
import shutil
import subprocess
import sys
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class Check:
    name: str
    status: str
    summary: str
    detail: str | None = None


def default_database() -> Path:
    override = os.environ.get("SUPER_MEM_DB")
    if override:
        return Path(override).expanduser()
    home = Path.home()
    system = platform.system()
    if system == "Windows":
        base = Path(os.environ.get("LOCALAPPDATA", home / "AppData" / "Local"))
    elif system == "Darwin":
        base = home / "Library" / "Application Support"
    else:
        base = Path(os.environ.get("XDG_DATA_HOME", home / ".local" / "share"))
    return base / "super-mem" / "memory.sqlite3"


def resolve_program(value: str) -> str | None:
    if os.path.dirname(value):
        candidate = Path(value).expanduser()
        return str(candidate.resolve()) if candidate.is_file() else None
    return shutil.which(value)


def run(command: list[str], *, cwd: Path | None = None, timeout: float = 5.0) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    environment.update(
        {
            "GIT_TERMINAL_PROMPT": "0",
            "GIT_OPTIONAL_LOCKS": "0",
        }
    )
    return subprocess.run(
        command,
        cwd=cwd,
        env=environment,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        check=False,
    )


def check_binary(binary: str) -> tuple[Check, str | None]:
    resolved = resolve_program(binary)
    if resolved is None:
        return Check("supermem", "fail", "supermem is not on PATH", f"requested executable: {binary}"), None
    try:
        result = run([resolved, "--version"])
    except (OSError, subprocess.TimeoutExpired) as error:
        return Check("supermem", "fail", "supermem could not be executed", str(error)), resolved
    output = (result.stdout or result.stderr).strip()
    if result.returncode != 0:
        return Check("supermem", "fail", f"supermem --version exited {result.returncode}", output or None), resolved
    return Check("supermem", "pass", output or "binary started", resolved), resolved


def nearest_existing_parent(path: Path) -> Path:
    candidate = path.expanduser().resolve(strict=False)
    if candidate.exists() and candidate.is_dir():
        return candidate
    candidate = candidate.parent
    while not candidate.exists() and candidate != candidate.parent:
        candidate = candidate.parent
    return candidate


def check_database_path(path: Path) -> Check:
    expanded = path.expanduser().resolve(strict=False)
    if expanded.exists():
        if expanded.is_dir():
            return Check("database_path", "fail", "database path is a directory", str(expanded))
        readable = os.access(expanded, os.R_OK)
        writable = os.access(expanded, os.W_OK)
        if readable and writable:
            return Check("database_path", "pass", "existing database is readable and writable", str(expanded))
        return Check("database_path", "fail", "existing database lacks read/write access", str(expanded))
    parent = nearest_existing_parent(expanded)
    if parent.is_dir() and os.access(parent, os.W_OK):
        return Check(
            "database_path",
            "pass",
            "database can be created under the nearest existing parent",
            f"database={expanded}; parent={parent}",
        )
    return Check("database_path", "fail", "database parent is not writable", f"database={expanded}; parent={parent}")


def check_git(cwd: Path) -> Check:
    git = shutil.which("git")
    if git is None:
        return Check(
            "git",
            "warn",
            "Git is not on PATH; repository-aware applicability will be unavailable",
        )
    try:
        version = run([git, "--version"], timeout=3)
        probe = run([git, "-C", str(cwd), "rev-parse", "--is-inside-work-tree"], timeout=3)
    except (OSError, subprocess.TimeoutExpired) as error:
        return Check("git", "warn", "Git probe did not complete", str(error))
    version_text = version.stdout.strip() or "git found"
    if probe.returncode == 0 and probe.stdout.strip() == "true":
        return Check("git", "pass", f"{version_text}; working tree detected", str(cwd.resolve()))
    return Check(
        "git",
        "warn",
        f"{version_text}; selected directory is not a readable Git working tree",
        str(cwd.resolve()),
    )


def check_cwd(cwd: Path) -> Check:
    if not cwd.exists():
        return Check("working_directory", "fail", "working directory does not exist", str(cwd))
    if not cwd.is_dir():
        return Check("working_directory", "fail", "working directory is not a directory", str(cwd))
    return Check("working_directory", "pass", "working directory is accessible", str(cwd.resolve()))


def check_doctor(binary: str, cwd: Path, timeout: float) -> Check:
    try:
        result = run([binary, "--json", "doctor", "--cwd", str(cwd)], timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as error:
        return Check("doctor", "fail", "doctor did not complete", str(error))
    output = result.stdout.strip()
    detail: str | None = None
    if output:
        try:
            payload: Any = json.loads(output)
            if isinstance(payload, dict):
                detail = f"JSON report keys: {', '.join(sorted(str(key) for key in payload)[:12])}"
        except json.JSONDecodeError:
            detail = "doctor did not emit valid JSON"
    if result.returncode == 0:
        return Check("doctor", "pass", "observational diagnostics passed", detail)
    stderr = result.stderr.strip()
    return Check("doctor", "fail", f"doctor exited {result.returncode}", stderr or detail)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="supermem", help="supermem executable")
    parser.add_argument("--cwd", type=Path, default=Path.cwd(), help="repository directory to probe")
    parser.add_argument("--db", type=Path, help="database path; defaults like the CLI")
    parser.add_argument("--doctor", action="store_true", help="also run the non-mutating doctor command")
    parser.add_argument("--doctor-timeout", type=float, default=15.0)
    parser.add_argument("--json", action="store_true", help="emit machine-readable JSON")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.doctor_timeout <= 0:
        print("error: --doctor-timeout must be positive", file=sys.stderr)
        return 2

    cwd = args.cwd.expanduser()
    checks: list[Check] = [check_cwd(cwd)]
    binary_check, resolved = check_binary(args.binary)
    checks.append(binary_check)
    checks.append(check_database_path(args.db or default_database()))
    if cwd.exists() and cwd.is_dir():
        checks.append(check_git(cwd))
    if args.doctor and resolved and cwd.exists() and cwd.is_dir():
        checks.append(check_doctor(resolved, cwd, args.doctor_timeout))

    if args.json:
        print(json.dumps({"checks": [asdict(check) for check in checks]}, indent=2, sort_keys=True))
    else:
        width = max(len(check.name) for check in checks)
        for check in checks:
            print(f"{check.status.upper():4}  {check.name:<{width}}  {check.summary}")
            if check.detail:
                print(f"      {'':<{width}}  {check.detail}")
        failures = sum(check.status == "fail" for check in checks)
        warnings = sum(check.status == "warn" for check in checks)
        print(f"\n{len(checks) - failures - warnings} passed, {warnings} warning(s), {failures} failure(s)")
    return 1 if any(check.status == "fail" for check in checks) else 0


if __name__ == "__main__":
    raise SystemExit(main())
