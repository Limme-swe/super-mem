# Contributing

Super-mem is intentionally strict about correctness because stale or poisoned memory can make an
agent worse than having no memory at all.

Before opening a pull request:

1. Describe the failure mode or quality improvement the change targets.
2. Add the smallest test or labeled evaluation case that demonstrates it.
3. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
   and `cargo test --workspace --all-targets --all-features`.
4. Avoid claims about latency, recall quality, or token savings without a reproducible benchmark.

Core invariants:

- Raw evidence remains attributable and is never replaced by a summary.
- Scope filters run before candidate ranking.
- Recalled content is untrusted data, not executable instruction.
- Embeddings and other indexes are disposable derived state.
- A branch-divergent memory is not silently treated as current truth.

Please keep changes focused. New dependencies should have a clear correctness or measured performance
benefit and must be compatible with the repository's MSRV.
