# Release process

1. Re-verify the Google sources in `SPEC_SUPPORT.md` and update its date.
2. Update `CHANGELOG.md` and the workspace version.
3. Run the four verification commands in the README with `--locked`.
4. Run `cargo audit` against the committed lockfile and resolve every vulnerability.
5. Smoke-test `audit`, `audit --strict`, and `fix` on the sample feeds.
6. Commit a clean worktree, create a signed `vX.Y.Z` tag, and push it.
7. The tag workflow creates a draft release, builds Linux/macOS/Windows archives and checksums, then publishes only after every platform succeeds.
8. Download each published archive and verify its checksum and `greenlit --version` before announcing it.

Do not publish from an uncommitted worktree or move an existing release tag.
