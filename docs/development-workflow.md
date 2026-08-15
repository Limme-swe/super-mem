# Development workflow

The repository keeps core invariants strict while making routine checks available through one cross-platform Python entry point.

## Prerequisites

- Rustup with the pinned toolchain from `rust-toolchain.toml`.
- Rust 1.88 for minimum-supported-version checks.
- Python 3.12 or newer for release and comfort tooling.
- Node.js 24 for adapter and retrieval-fixture work.
- Git on `PATH`.

The pinned development and release toolchain may be newer than the minimum supported Rust version. Changes must continue to compile under the MSRV jobs.

## Discover available checks

```sh
python scripts/dev.py --list
```

Common targets:

| Target | Purpose |
| --- | --- |
| `quick` | Formatting, compile check, Python tests, docs, and fixture validation. |
| `rust` | Formatting, Clippy with warnings denied, and the full Rust test suite. |
| `scripts` | Python syntax/tests, documentation links, and fixture validators. |
| `docs` | Local Markdown-link validation. |
| `package` | Dry package construction for both crates. |
| `full` | Review-ready local validation across the above areas. |

Run commands without executing them:

```sh
python scripts/dev.py full --dry-run
```

Continue after failures to collect a complete local report:

```sh
python scripts/dev.py full --keep-going
```

## Focused commands

Rust formatting and linting:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Tests:

```sh
cargo test --workspace --all-targets --all-features
python -m unittest discover -s scripts/tests -p "test_*.py"
node scripts/validate-eval-fixture.mjs
node scripts/validate-retrieval-fixture.mjs
```

Documentation:

```sh
python scripts/check_docs.py
python scripts/check_docs.py docs README.md
```

Package validation:

```sh
cargo package -p super-mem-core --locked --no-verify
cargo package -p super-mem --locked --no-verify \
  --config 'patch.crates-io.super-mem-core.path="crates/super-mem-core"'
```

## Testing scripts safely

Installer tests use local release fixtures and never contact GitHub. The POSIX installer test builds a temporary archive, verifies the checksum path, installs a fake binary, and confirms tampered content is rejected.

PowerShell scripts are parsed on Windows CI. Destructive uninstall behavior must remain dry-run capable, preserve memory by default, and require an explicit confirmation flag for data removal.

The installed-binary smoke test always supplies explicit temporary database paths and removes `SUPER_MEM_DB`, `SUPER_MEM_NAMESPACE`, and `SUPER_MEM_WORKSPACE` from the child environment.

## Documentation rules

`check_docs.py` validates local Markdown targets across the repository, ignores external URLs and fenced examples, and rejects links that escape the repository root. It intentionally does not make network requests or claim that external sites are reachable.

Keep user workflows concrete and copyable. When behavior changes, update the narrowest relevant guide and the README navigation rather than duplicating a second reference.

## Change discipline

- Preserve raw evidence and provenance.
- Apply hard scope filters before ranking or graph expansion.
- Keep recalled text untrusted.
- Keep SQLite canonical and derived indexes rebuildable.
- Distinguish compatible, stale, and branch-divergent evidence.
- Keep model calls and downloads out of critical write and recall paths.
- Add focused regression coverage for the affected invariant.
- Avoid unrelated refactors and speculative abstractions.
- Do not publish latency, quality, or token-saving claims without a reproducible workload, hardware, and before/after results.

## Before opening a pull request

```sh
python scripts/dev.py full
```

Then verify:

1. the failure mode or workflow improvement is clearly stated;
2. storage, snapshot, Git, MCP, and compatibility effects are described;
3. tests are focused and reproducible;
4. docs and changelog are updated when behavior changes;
5. no generated files, databases, exports, secrets, or local paths were added;
6. `git diff --check` is clean.

See [Contributing](../CONTRIBUTING.md) for repository policy and [Architecture](architecture.md) for system boundaries.
