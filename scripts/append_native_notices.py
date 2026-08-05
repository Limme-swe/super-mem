#!/usr/bin/env python3
"""Append exact Rust and native-library notices to a Cargo license report."""

from __future__ import annotations

import argparse
from pathlib import Path


def append_section(report: Path, heading: str, source: Path) -> None:
    if not source.is_file() or source.stat().st_size == 0:
        raise SystemExit(f"required native license notice is missing: {source}")
    content = source.read_bytes()
    with report.open("ab") as output:
        output.write(f"\n\n{'=' * 79}\n{heading}\n{'=' * 79}\n\n".encode())
        output.write(content)
        if not content.endswith(b"\n"):
            output.write(b"\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--rust-sysroot", type=Path)
    parser.add_argument("--musl-notice", type=Path)
    arguments = parser.parse_args()

    if not arguments.report.is_file() or arguments.report.stat().st_size == 0:
        raise SystemExit(f"Cargo license report is missing: {arguments.report}")
    if arguments.rust_sysroot is None and arguments.musl_notice is None:
        raise SystemExit("at least one native notice source is required")

    if arguments.rust_sysroot is not None:
        rust_docs = arguments.rust_sysroot / "share" / "doc" / "rust"
        for name in ("COPYRIGHT", "LICENSE-APACHE", "LICENSE-MIT"):
            append_section(
                arguments.report,
                f"Rust toolchain used for this release — {name}",
                rust_docs / name,
            )
    if arguments.musl_notice is not None:
        append_section(
            arguments.report,
            "musl C library — copyright and license notice",
            arguments.musl_notice,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
