#!/usr/bin/env python3
"""Validate release metadata and emit values for GitHub Actions."""

from __future__ import annotations

import argparse
import json
import re
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SEMVER = re.compile(
    r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)"
)


def load_toml(path: Path) -> dict:
    with path.open("rb") as source:
        return tomllib.load(source)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--github-output", type=Path)
    arguments = parser.parse_args()

    release = load_toml(ROOT / ".github/release.toml")
    version = release.get("version")
    if not isinstance(version, str) or SEMVER.fullmatch(version) is None:
        raise SystemExit(".github/release.toml contains an invalid semantic version")

    workspace = load_toml(ROOT / "Cargo.toml")
    workspace_version = workspace["workspace"]["package"]["version"]
    if workspace_version != version:
        raise SystemExit(
            f"release version {version} does not match Cargo workspace {workspace_version}"
        )

    cli = load_toml(ROOT / "crates/super-mem-cli/Cargo.toml")
    core_dependency = cli["dependencies"]["super-mem-core"]["version"]
    if core_dependency != version:
        raise SystemExit(
            f"CLI core dependency {core_dependency} does not match release {version}"
        )

    lock = load_toml(ROOT / "Cargo.lock")
    local_versions = {
        package["name"]: package["version"]
        for package in lock["package"]
        if package["name"] in {"super-mem", "super-mem-core"}
    }
    if local_versions != {"super-mem": version, "super-mem-core": version}:
        raise SystemExit(f"Cargo.lock local package versions do not match {version}")

    json_manifests = (
        ROOT / "adapters/opencode/package.json",
        ROOT / "adapters/pi/package.json",
        ROOT / "adapters/codex/.codex-plugin/plugin.json",
    )
    for manifest in json_manifests:
        declared = json.loads(manifest.read_text(encoding="utf-8")).get("version")
        if declared != version:
            raise SystemExit(
                f"{manifest.relative_to(ROOT)} version {declared!r} does not match {version}"
            )

    notes = ROOT / "docs/releases" / f"v{version}.md"
    if not notes.is_file():
        raise SystemExit(f"release notes are missing: {notes.relative_to(ROOT)}")
    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    if f"## [{version}]" not in changelog:
        raise SystemExit(f"CHANGELOG.md has no {version} section")

    values = {
        "version": version,
        "tag": f"v{version}",
        "notes": notes.relative_to(ROOT).as_posix(),
    }
    if arguments.github_output:
        with arguments.github_output.open("a", encoding="utf-8", newline="\n") as output:
            for key, value in values.items():
                output.write(f"{key}={value}\n")
    else:
        for key, value in values.items():
            print(f"{key}={value}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
