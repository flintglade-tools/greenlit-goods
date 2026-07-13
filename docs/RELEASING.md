# Release process

1. Re-verify the Google sources in `SPEC_SUPPORT.md` and update its date.
2. Update `CHANGELOG.md` and the workspace version.
3. Run the four verification commands in the README with `--locked`.
4. Run `./action/tests/entrypoint_test.sh`, build the root `Dockerfile`, and run the clean and strict-failure action smoke tests documented in CI.
5. Run `cargo audit` against the committed lockfile and resolve every vulnerability.
6. Smoke-test `audit`, `audit --strict`, and `fix` on the sample feeds.
7. Review the immutable Rust and Debian base-image digests in `Dockerfile`; update them only after rebuilding and rerunning the action tests.
8. Commit a clean worktree, create an SSH-signed `vX.Y.Z` tag with the Flintglade release key, verify it locally, and push it. The workflow verifies it again against `.github/allowed_signers`.
9. The tag workflow creates the draft if needed, reruns the complete verification and RustSec gates, and builds architecture-labelled Linux x86_64, macOS arm64, macOS x86_64, and Windows x86_64 archives.
10. The workflow uploads assets with overwrite-safe rerun behavior and publishes only after all four platform builds succeed.
11. Download each published archive, verify its checksum and bundled documentation, and run `greenlit --version` wherever the host architecture permits before announcing it.
12. For a Marketplace release, complete the owner-only web steps in [MARKETPLACE.md](MARKETPLACE.md) after the ordinary release is healthy.

Do not publish from an uncommitted worktree or move an existing release tag. A failed workflow may be rerun against the same immutable tag; it must never require deleting the draft or its assets.
