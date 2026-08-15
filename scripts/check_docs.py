#!/usr/bin/env python3
"""Validate local links in repository Markdown without third-party packages."""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import unquote, urlsplit

LINK = re.compile(r"(?<!!)\[[^\]]*\]\(([^)]+)\)|!\[[^\]]*\]\(([^)]+)\)")
FENCE = re.compile(r"^\s*(`{3,}|~{3,})")
SCHEMES = {"http", "https", "mailto", "tel", "data"}
DEFAULT_EXCLUDES = {".git", "target", "node_modules", ".venv", "venv"}


@dataclass(frozen=True)
class Problem:
    file: Path
    line: int
    target: str
    reason: str

    def render(self, root: Path) -> str:
        try:
            source = self.file.relative_to(root)
        except ValueError:
            source = self.file
        return f"{source}:{self.line}: {self.reason}: {self.target}"


def markdown_files(root: Path, requested: list[str]) -> list[Path]:
    if requested:
        files: list[Path] = []
        for raw in requested:
            path = (root / raw).resolve() if not Path(raw).is_absolute() else Path(raw).resolve()
            if path.is_dir():
                files.extend(path.rglob("*.md"))
            elif path.suffix.lower() == ".md":
                files.append(path)
            else:
                raise ValueError(f"not a Markdown file or directory: {raw}")
    else:
        files = list(root.rglob("*.md"))

    unique: list[Path] = []
    seen: set[Path] = set()
    for path in sorted(files):
        if any(part in DEFAULT_EXCLUDES for part in path.parts):
            continue
        resolved = path.resolve()
        if resolved not in seen:
            seen.add(resolved)
            unique.append(resolved)
    return unique


def strip_fenced_blocks(text: str) -> list[tuple[int, str]]:
    visible: list[tuple[int, str]] = []
    fence: str | None = None
    for number, line in enumerate(text.splitlines(), start=1):
        match = FENCE.match(line)
        if match:
            marker = match.group(1)
            if fence is None:
                fence = marker[0]
            elif marker[0] == fence:
                fence = None
            continue
        if fence is None:
            visible.append((number, line))
    return visible


def extract_target(match: re.Match[str]) -> str:
    raw = match.group(1) or match.group(2) or ""
    raw = raw.strip()
    if raw.startswith("<") and raw.endswith(">"):
        raw = raw[1:-1].strip()
    # Markdown permits an optional title after a whitespace separator.
    if " " in raw and not raw.startswith(("http://", "https://")):
        raw = raw.split(maxsplit=1)[0]
    return raw


def validate_file(root: Path, source: Path) -> list[Problem]:
    problems: list[Problem] = []
    try:
        text = source.read_text(encoding="utf-8")
    except (OSError, UnicodeError) as error:
        return [Problem(source, 1, str(source), f"cannot read Markdown ({error})")]

    for line_number, line in strip_fenced_blocks(text):
        for match in LINK.finditer(line):
            raw = extract_target(match)
            if not raw or raw.startswith("#"):
                continue
            parsed = urlsplit(raw)
            if parsed.scheme.lower() in SCHEMES or raw.startswith("//"):
                continue
            target_text = unquote(parsed.path)
            if not target_text:
                continue
            if target_text.startswith("/"):
                candidate = (root / target_text.lstrip("/")).resolve()
            else:
                candidate = (source.parent / target_text).resolve()
            try:
                candidate.relative_to(root)
            except ValueError:
                problems.append(Problem(source, line_number, raw, "link escapes repository root"))
                continue
            if not candidate.exists():
                problems.append(Problem(source, line_number, raw, "missing local target"))
    return problems


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*", help="Markdown files or directories; defaults to the repository")
    parser.add_argument("--root", type=Path, default=Path.cwd(), help="repository root")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    root = args.root.resolve()
    try:
        files = markdown_files(root, args.paths)
    except ValueError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    if not files:
        print("error: no Markdown files found", file=sys.stderr)
        return 2

    problems = [problem for source in files for problem in validate_file(root, source)]
    if problems:
        for problem in problems:
            print(problem.render(root), file=sys.stderr)
        print(f"documentation check failed: {len(problems)} broken local link(s)", file=sys.stderr)
        return 1
    print(f"documentation check passed: {len(files)} Markdown file(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
