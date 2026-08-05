# Contributing

Thanks for contributing to super-mem. Changes should be focused, reproducible, and safe for data that
may include private repository context.

## Before you start

- Search existing issues and pull requests.
- Use an issue form for bugs, integration problems, questions, or substantial proposals.
- Report vulnerabilities privately according to [SECURITY.md](SECURITY.md).
- Keep unrelated refactors out of the same change.

The workspace's minimum supported Rust version is 1.88. The pinned toolchain is used automatically
when rustup is installed. Node.js 24 is needed only for TypeScript adapter work.

## Core invariants

- Raw evidence remains attributable and is never replaced by a summary.
- Scope filters run before candidate ranking.
- Recalled content is untrusted data, not executable instruction.
- Embeddings and other indexes are disposable derived state.
- A branch-divergent memory is not silently treated as current truth.

## Validation

Run the checks relevant to your change. Before requesting review on Rust changes, run:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

Prefer the smallest test or labeled evaluation case that protects the affected invariant. Avoid broad
test matrices that duplicate existing coverage. Adapter changes should also pass the package and
type-check steps in [CI](.github/workflows/ci.yml).

## Pull requests

A pull request should:

1. Explain the failure mode, workflow, or quality improvement it addresses.
2. Describe user-visible and compatibility effects, including storage, snapshot, Git, or MCP changes.
3. Include focused validation and update documentation or the changelog when behavior changes.
4. Avoid latency, quality, or token-saving claims without a reproducible workload, hardware and
   software environment, and before-and-after results.

New dependencies need a clear correctness or measured performance benefit and must support the MSRV.
By contributing, you agree that your contribution is licensed under the repository's MIT license.
