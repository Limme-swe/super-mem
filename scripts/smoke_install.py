#!/usr/bin/env python3
"""Smoke-test an installed supermem binary against temporary databases only."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from dataclasses import asdict, dataclass
from pathlib import Path


@dataclass(frozen=True)
class StepResult:
    name: str
    exit_code: int | None
    stdout: str
    stderr: str


def resolve_binary(value: str) -> str:
    if os.path.dirname(value):
        candidate = Path(value).expanduser()
        if candidate.is_file():
            return str(candidate.resolve())
    else:
        resolved = shutil.which(value)
        if resolved:
            return resolved
    raise RuntimeError(f"supermem executable not found: {value}")


def execute(name: str, command: list[str], *, cwd: Path, timeout: float) -> StepResult:
    environment = os.environ.copy()
    # The explicit --db arguments below must win, but removing these variables also
    # proves the smoke test cannot accidentally reuse the user's configured scope.
    for key in ("SUPER_MEM_DB", "SUPER_MEM_NAMESPACE", "SUPER_MEM_WORKSPACE"):
        environment.pop(key, None)
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
        return StepResult(name, result.returncode, result.stdout[-8192:], result.stderr[-8192:])
    except subprocess.TimeoutExpired as error:
        stdout = error.stdout.decode(errors="replace") if isinstance(error.stdout, bytes) else (error.stdout or "")
        stderr = error.stderr.decode(errors="replace") if isinstance(error.stderr, bytes) else (error.stderr or "")
        return StepResult(name, None, stdout[-8192:], (stderr + "\ncommand timed out")[-8192:])
    except OSError as error:
        return StepResult(name, None, "", str(error))


def workflow(binary: str, root: Path, timeout: float) -> list[StepResult]:
    primary = root / "primary.sqlite3"
    restored = root / "restored.sqlite3"
    snapshot = root / "memory.jsonl"
    commands = [
        ("version", [binary, "--version"]),
        ("init", [binary, "--db", str(primary), "init"]),
        (
            "remember",
            [
                binary,
                "--db",
                str(primary),
                "remember",
                "--kind",
                "decision",
                "--body",
                "Smoke-test memory created in an isolated temporary database.",
                "--canonical-key",
                "super-mem-install-smoke",
            ],
        ),
        (
            "recall",
            [
                binary,
                "--db",
                str(primary),
                "recall",
                "--query",
                "isolated installation smoke test",
                "--token-budget",
                "256",
            ],
        ),
        ("export", [binary, "--db", str(primary), "export", "--output", str(snapshot)]),
        ("import", [binary, "--db", str(restored), "import", str(snapshot)]),
        ("restored_status", [binary, "--db", str(restored), "--json", "status"]),
        ("restored_doctor", [binary, "--db", str(restored), "--json", "doctor", "--cwd", str(root)]),
    ]
    results: list[StepResult] = []
    for name, command in commands:
        result = execute(name, command, cwd=root, timeout=timeout)
        results.append(result)
        if result.exit_code != 0:
            break
        if name == "export" and not snapshot.is_file():
            results.append(StepResult("export_file", 1, "", f"snapshot was not created: {snapshot}"))
            break
    return results


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="supermem", help="supermem executable")
    parser.add_argument("--work-dir", type=Path, help="use and retain this empty directory")
    parser.add_argument("--timeout", type=float, default=20.0, help="per-command timeout")
    parser.add_argument("--json", action="store_true", help="emit machine-readable results")
    return parser.parse_args(argv)


def run_in_root(binary: str, root: Path, timeout: float, json_output: bool) -> int:
    root.mkdir(parents=True, exist_ok=True)
    if any(root.iterdir()):
        print(f"error: smoke-test directory must be empty: {root}", file=sys.stderr)
        return 2
    results = workflow(binary, root, timeout)
    passed = all(result.exit_code == 0 for result in results) and len(results) == 8
    if json_output:
        print(json.dumps({"passed": passed, "steps": [asdict(result) for result in results]}, indent=2))
    else:
        for result in results:
            state = "PASS" if result.exit_code == 0 else "FAIL"
            code = "timeout" if result.exit_code is None else str(result.exit_code)
            print(f"{state:4}  {result.name} (exit {code})")
            if result.exit_code != 0:
                detail = (result.stderr or result.stdout).strip()
                if detail:
                    print(f"      {detail}")
        print("\nInstallation smoke test passed." if passed else "\nInstallation smoke test failed.")
    return 0 if passed else 1


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.timeout <= 0:
        print("error: --timeout must be positive", file=sys.stderr)
        return 2
    try:
        binary = resolve_binary(args.binary)
    except RuntimeError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    if args.work_dir:
        return run_in_root(binary, args.work_dir.expanduser().resolve(strict=False), args.timeout, args.json)
    with tempfile.TemporaryDirectory(prefix="super-mem-smoke-") as directory:
        return run_in_root(binary, Path(directory), args.timeout, args.json)


if __name__ == "__main__":
    raise SystemExit(main())
