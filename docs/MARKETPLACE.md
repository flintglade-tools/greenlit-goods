# GitHub Marketplace release

Greenlit Goods is both an ordinary offline CLI/library and, starting with version 0.2.0, an honest CI wrapper around that same auditor. The Marketplace listing must describe the audit action only; it must not imply Google affiliation, live Merchant Center access, or checks outside the documented feed-local support boundary.

## Eligibility

GitHub's current Action requirements are:

- a public repository;
- exactly one `action.yml` or `action.yaml` at the repository root;
- a unique metadata `name` that is not a reserved GitHub name, account name, feature, or Marketplace category;
- a repository limited to the metadata, code, and files needed by that action;
- an accepted GitHub Marketplace Developer Agreement.

This repository is public and contains one root `action.yml`. Its CLI engine, Docker entrypoint, tests, samples, documentation, and release files all directly support the one feed-audit action. A public Marketplace search found no existing listing named **Greenlit Goods Feed Audit** during preparation, but GitHub's release form is the authoritative final uniqueness check.

Authoritative references:

- [Publishing actions in GitHub Marketplace](https://docs.github.com/en/actions/how-tos/create-and-publish-actions/publish-in-github-marketplace)
- [GitHub Action metadata syntax](https://docs.github.com/en/actions/reference/workflows-and-actions/metadata-syntax)
- [Managing custom actions](https://docs.github.com/en/actions/how-tos/create-and-publish-actions/manage-custom-actions)

## Release checklist

1. Re-run the complete release gates in [RELEASING.md](RELEASING.md), including the action contract and Docker smoke tests.
2. Replace `[Unreleased]` in `CHANGELOG.md` with the release version and date.
3. Commit a clean tree and create the verified SSH-signed `vX.Y.Z` tag required by the release workflow.
4. Wait for all platform archives and checksums to publish successfully.
5. As an owner of `flintglade-tools`, edit that GitHub release in the web UI and select **Publish this Action to the GitHub Marketplace**.
6. Accept the GitHub Marketplace Developer Agreement if the organization has not already done so.
7. Resolve any metadata warning shown by GitHub. Confirm that the unique name is still available, then choose the most accurate live categories (normally continuous integration/code quality for this action).
8. Publish or update the release. GitHub requires two-factor authentication for this operation.
9. Open the Marketplace page in a signed-out session and copy the exact generated `uses:` example into a throwaway workflow. Confirm a clean feed passes and `samples/broken.xml` fails with default strict mode.

Agreement acceptance, category selection, the Marketplace checkbox, and the two-factor-authenticated publish confirmation are intentionally owner-performed web steps. They cannot be represented by repository files or the GitHub CLI release workflow.
