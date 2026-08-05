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


def first_notice(candidates: tuple[Path, ...]) -> Path:
    for candidate in candidates:
        if candidate.is_file() and candidate.stat().st_size > 0:
            return candidate
    rendered = "\n  ".join(str(candidate) for candidate in candidates)
    raise SystemExit(f"required native license notice is missing; checked:\n  {rendered}")


def rust_license_notices(rust_docs: Path) -> tuple[Path, ...]:
    license_directory = rust_docs / "licenses"
    if license_directory.is_dir():
        notices = tuple(sorted(license_directory.glob("*.txt"), key=lambda path: path.name))
        if not notices:
            raise SystemExit(f"Rust license directory contains no notices: {license_directory}")
        for notice in notices:
            if not notice.is_file() or notice.stat().st_size == 0:
                raise SystemExit(f"required native license notice is missing: {notice}")
        names = {notice.name for notice in notices}
        required = {"Apache-2.0.txt", "MIT.txt"}
        missing = sorted(required - names)
        if missing:
            raise SystemExit(
                f"Rust license directory is missing required notices: {', '.join(missing)}"
            )
        return notices
    return (
        first_notice((rust_docs / "LICENSE-APACHE",)),
        first_notice((rust_docs / "LICENSE-MIT",)),
    )


def append_rust_notices(report: Path, rust_sysroot: Path) -> None:
    rust_docs = rust_sysroot / "share" / "doc" / "rust"
    copyright_notice = first_notice(
        (
            rust_docs / "COPYRIGHT-library.html",
            rust_docs / "COPYRIGHT.html",
            rust_docs / "html" / "COPYRIGHT-library.html",
            rust_docs / "html" / "COPYRIGHT.html",
        )
    )
    append_section(
        report,
        f"Rust standard library and runtime — {copyright_notice.name}",
        copyright_notice,
    )
    for source in rust_license_notices(rust_docs):
        append_section(
            report,
            f"Rust toolchain used for this release — {source.name}",
            source,
        )


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
        append_rust_notices(arguments.report, arguments.rust_sysroot)
    if arguments.musl_notice is not None:
        append_section(
            arguments.report,
            "musl C library — copyright and license notice",
            arguments.musl_notice,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
