# Contributing

Keep changes narrow and attach evidence to behavior changes.

1. Add or update a test that demonstrates the observable requirement.
2. Make the smallest implementation change that passes it.
3. Run `cargo fmt --all -- --check`.
4. Run `cargo clippy --workspace --all-targets --locked -- -D warnings`.
5. Run `cargo test --workspace --locked` and `cargo build --workspace --release --locked`.

Rule changes must cite an authoritative Google specification page and update `docs/SPEC_SUPPORT.md` plus the verification date. Heuristics must use the heuristic finding basis and must not claim Google disapproval. Parser changes must preserve the fail-closed rewrite contract and include adversarial coverage.

Never include real merchant feeds, credentials, personal data, or secret-bearing logs in tests or issues.
