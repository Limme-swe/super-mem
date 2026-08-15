#!/usr/bin/env python3
"""Run the repository's common development checks through one cross-platform entry point."""

from __future__ import annotations

import argparse
import os
import shlex
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
PYTHON = sys.executable


@dataclass(frozen=True)
class Step:
    label: str
    command: tuple[str, ...]


FMT = Step("Rust formatting", ("cargo", "fmt", "--all", "--check"))
CHECK = Step(
    "Rust compile check",
    ("cargo", "check", "--workspace", "--all-targets", "--all-features", "--locked"),
)
CLIPPY = Step(
    "Rust lint",
    ("cargo", "clippy", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"),
)
TEST = Step(
    "Rust tests",
    ("cargo", "test", "--workspace", "--all-targets", "--all-features"),
)
PY_COMPILE = Step("Python syntax", (PYTHON, "-m", "compileall", "-q", "scripts"))
PY_TEST = Step(
    "Python tests",
    (PYTHON, "-m", "unittest", "discover", "-s", "scripts/tests", "-p", "test_*.py"),
)
DOCS = Step("Documentation links", (PYTHON, "scripts/check_docs.py"))
EVAL = Step("Evaluation fixture", ("node", "scripts/validate-eval-fixture.mjs"))
RETRIEVAL = Step("Retrieval fixture", ("node", "scripts/validate-retrieval-fixture.mjs"))
PACKAGE_CORE = Step(
    "Package core crate",
    ("cargo", "package", "-p", "super-mem-core", "--locked", "--no-verify"),
)
PACKAGE_CLI = Step(
    "Package CLI crate",
    (
        "cargo",
        "package",
        "-p",
        "super-mem",
        "--locked",
        "--no-verify",
        "--config",
        'patch.crates-io.super-mem-core.path="crates/super-mem-core"',
    ),
)

TARGETS: dict[str, tuple[Step, ...]] = {
    "fmt": (FMT,),
    "quick": (FMT, CHECK, PY_COMPILE, PY_TEST, DOCS, EVAL, RETRIEVAL),
    "rust": (FMT, CLIPPY, TEST),
    "scripts": (PY_COMPILE, PY_TEST, DOCS, EVAL, RETRIEVAL),
    "docs": (DOCS,),
    "package": (PACKAGE_CORE, PACKAGE_CLI),
    "full": (
        FMT,
        CLIPPY,
        TEST,
        PY_COMPILE,
        PY_TEST,
        DOCS,
        EVAL,
        RETRIEVAL,
        PACKAGE_CORE,
        PACKAGE_CLI,
    ),
}


def display(command: tuple[str, ...]) -> str:
    if os.name == "nt":
        return subprocess.list2cmdline(command)
    return shlex.join(command)


def run_steps(steps: tuple[Step, ...], *, dry_run: bool, keep_going: bool) -> int:
    failures: list[tuple[Step, int]] = []
    for index, step in enumerate(steps, start=1):
        print(f"[{index}/{len(steps)}] {step.label}")
        print(f"  $ {display(step.command)}")
        if dry_run:
            continue
        try:
            result = subprocess.run(step.command, cwd=ROOT, check=False)
        except FileNotFoundError:
            print(f"error: required command not found: {step.command[0]}", file=sys.stderr)
            failures.append((step, 127))
            if not keep_going:
                break
            continue
        if result.returncode != 0:
            failures.append((step, result.returncode))
            if not keep_going:
                break
    if failures:
        print("\nFailed steps:", file=sys.stderr)
        for step, code in failures:
            print(f"- {step.label} (exit {code})", file=sys.stderr)
        return 1
    if not dry_run:
        print(f"\nAll {len(steps)} step(s) passed.")
    return 0


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("target", nargs="?", choices=sorted(TARGETS), default="quick")
    parser.add_argument("--dry-run", action="store_true", help="print commands without executing them")
    parser.add_argument("--keep-going", action="store_true", help="run remaining steps after a failure")
    parser.add_argument("--list", action="store_true", help="list targets and exit")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.list:
        for name, steps in TARGETS.items():
            print(f"{name:8} {len(steps)} step(s): {', '.join(step.label for step in steps)}")
        return 0
    return run_steps(TARGETS[args.target], dry_run=args.dry_run, keep_going=args.keep_going)


if __name__ == "__main__":
    raise SystemExit(main())
