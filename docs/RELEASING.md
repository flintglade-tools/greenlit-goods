# Release process

1. Re-verify the Google sources in `SPEC_SUPPORT.md` and update its date.
2. Update `CHANGELOG.md` and the workspace version.
3. Run the four verification commands in the README with `--locked`.
4. Run `cargo audit` against the committed lockfile and resolve every vulnerability.
5. Smoke-test `audit`, `audit --strict`, and `fix` on the sample feeds.
6. Commit a clean worktree, create an SSH-signed `vX.Y.Z` tag with the Flintglade release key, verify it locally, and push it. The workflow verifies it again against `.github/allowed_signers`.
7. The tag workflow creates the draft if needed, reruns the complete verification and RustSec gates, and builds architecture-labelled Linux x86_64, macOS arm64, macOS x86_64, and Windows x86_64 archives.
8. The workflow uploads assets with overwrite-safe rerun behavior and publishes only after all four platform builds succeed.
9. Download each published archive, verify its checksum and bundled documentation, and run `greenlit --version` wherever the host architecture permits before announcing it.

Do not publish from an uncommitted worktree or move an existing release tag. A failed workflow may be rerun against the same immutable tag; it must never require deleting the draft or its assets.
