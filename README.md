# Greenlit Goods

[![CI](https://github.com/flintglade-tools/greenlit-goods/actions/workflows/ci.yml/badge.svg)](https://github.com/flintglade-tools/greenlit-goods/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/flintglade-tools/greenlit-goods)](https://github.com/flintglade-tools/greenlit-goods/releases/latest)

Greenlit Goods is a local Rust CLI and library for auditing Google Merchant Center product feeds. It reads RSS/XML and delimited text, explains checkable feed problems, and can make a deliberately small set of deterministic repairs.

This release is an independent feed preflight tool. It is not affiliated with Google, does not call Google services, and cannot predict account-, landing-page-, image-, policy-, or crawl-dependent decisions.

## Release status

Version `0.1.1` is the current published CLI/library release. The `0.2.0` development line adds the same offline auditor as a GitHub Action; it becomes consumable from a stable tag after that release is published. JSON output is versioned with `schema_version: 1`; incompatible JSON changes require a schema-version change.

The implementation was verified with Rust 1.96.1. The manifest requires Rust 1.96 or newer.

## Build and use

```console
cargo build --release --locked

# Audit. Exit 0 means the audit ran successfully.
target/release/greenlit audit samples/clean.xml
target/release/greenlit audit feed.csv --json

# Make strict mode a CI gate. Exit 1 means red products or incomplete input.
target/release/greenlit audit feed.xml --strict

# Apply conservative fixes to a new file. Existing files are never replaced.
target/release/greenlit fix feed.xml --output feed.fixed.xml
```

On Windows the binary is `target\release\greenlit.exe`.

Prebuilt, checksum-paired Linux x86_64, macOS arm64/x86_64, and Windows x86_64 archives are available from [GitHub Releases](https://github.com/flintglade-tools/greenlit-goods/releases). Verify the adjacent `.sha256` file before running an archive; for example:

```console
sha256sum -c greenlit-v0.1.1-linux-x86_64.tar.gz.sha256
```

Useful options:

- `--country US` selects the two-letter target country used by conditional rules.
- `--destination shopping-ads|free-listings` selects the Google surface.
- `--assumed-monthly-sales N` controls the explicitly labeled revenue estimate.
- `--format xml|csv` overrides format detection.
- `--json` emits the full structured report or fix log.
- `--no-color` disables ANSI styling.

Exit codes are stable for v0.1: `0` for a completed command, `1` for an audit `--strict` failure, and `2` for invalid arguments, unreadable input, unsafe rewrite, or another command error.

## GitHub Action

Releases containing the root `action.yml` (starting with `v0.2.0`) can audit a feed as a Linux CI gate. The action builds and runs the CLI at the selected action ref in an isolated Docker image. Building the image pulls the pinned base images and locked Rust dependencies; the auditor itself makes no network requests after launch and needs no secrets or write permissions.

```yaml
name: Product feed

on:
  pull_request:

permissions:
  contents: read

jobs:
  audit-feed:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7
      - uses: flintglade-tools/greenlit-goods@v0.2.0
        with:
          feed: path/to/products.xml
          country: US
          destination: shopping-ads
```

Inputs:

| Input | Required | Default | Purpose |
| --- | --- | --- | --- |
| `feed` | yes | — | Repository-relative path to an RSS/XML, CSV, or TSV feed |
| `format` | no | auto-detect | Optional `xml` or `csv` override |
| `country` | no | `US` | Two-letter target country for conditional rules |
| `destination` | no | `shopping-ads` | `shopping-ads` or `free-listings` |
| `assumed-monthly-sales` | no | `1` | Explicit revenue-at-risk assumption |
| `strict` | no | `true` | Fail on red products or incomplete input |
| `json` | no | `false` | Emit the schema-versioned JSON report |

The action has no outputs, secrets, or environment-variable requirements. Set `strict: "false"` for report-only use. Use `format` only when detection is not suitable.

Docker actions run on Linux runners. macOS and Windows users can use the release archives or build the CLI locally. For supply-chain-sensitive workflows, pin the action to a reviewed full commit SHA instead of a movable tag.

## What is checked

The rules cover feed-local parts of the published product data specification, including core required fields, documented length and enum constraints, strict price/currency syntax, GTIN checksums, identifier combinations, duplicate IDs, availability-date logic, selected apparel requirements, and several clearly labeled listing-quality heuristics.

Every JSON finding declares its basis as `google_specification`, `greenlit_heuristic`, or `feed_structure`. Normative findings include the source URL and verification date. The detailed coverage matrix and known exclusions are in [docs/SPEC_SUPPORT.md](docs/SPEC_SUPPORT.md).

The Greenlight Score is a product-level prioritization metric created by Greenlit Goods, not a Google metric. Incomplete feeds receive `null`/`N/A`, never a deceptively precise score.

## Revenue-at-risk estimate

Revenue at risk is an estimate, not a forecast. It multiplies a valid affected product price by the stated monthly-sales assumption, weighting red products at 100% and yellow products at 30%. Invalid or missing prices are excluded and counted. Different currencies are reported separately and are never added together.

## Conservative fix contract

`greenlit fix` only:

- trims leading/trailing whitespace on known scalar fields;
- canonicalizes recognized enum variants;
- canonicalizes an unambiguous amount plus three-letter currency to two decimal places.

It does not invent identifiers, brands, currencies, categories, or apparel data. It does not rewrite prose, strip HTML, or change capitalization. Every change appears in the fix log.

XML rewriting is intentionally narrower than XML auditing. The command refuses to rewrite if preserving the source cannot be proven within the supported flat RSS representation—for example nested product structures, unsupported channel metadata, comments or processing instructions, custom namespaces, Atom entries, element attributes, repeated ambiguous values, malformed/truncated XML, uncertain source decoding, or a missing Google namespace. Output is written through a same-directory temporary file and committed without clobbering an existing path.

CSV/TSV output preserves the detected delimiter, source header labels/order, and all represented columns. Duplicate headers use disambiguated internal keys so their values are never silently merged, then retain their original labels on output.

## Safety limits

To bound local resource use, v0.1 rejects inputs over 64 MiB, more than 250,000 products, fields over 1,000,000 bytes, XML nesting deeper than 64 elements, or XML elements with more than 256 attributes.

## Architecture

- `engine/` is the I/O-free audit/fix library.
- `cli/` owns file access, atomic no-clobber output, rendering, and exit codes.
- `samples/` contains deterministic example feeds.

## Verification

Run the same gates as CI:

```console
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --release --locked
```

The test suite includes unit, adversarial parser, specification, rewrite-safety, typed-validation, round-trip, and real-binary CLI contract tests. CI runs these gates on Linux, macOS, and Windows and runs RustSec dependency auditing.

## Limitations

- No network requests are made, so landing-page availability/content, image properties, account state, policy enforcement, inventory reconciliation, and Google API diagnostics are out of scope.
- Nested shipping and tax structures can be audited only as unsupported structure in v0.1; they are never rewritten.
- Apparel keyword inference and placeholder-image checks are heuristics, not assertions about Google enforcement.
- URL checks are syntax checks, not reachability or ownership checks.
- The supported specification is a dated snapshot. Re-verify it before each release.

See [SECURITY.md](SECURITY.md), [CONTRIBUTING.md](CONTRIBUTING.md), [CHANGELOG.md](CHANGELOG.md), and [docs/RELEASING.md](docs/RELEASING.md) for project operations.

### Resource limits

Feeds are limited to 64 MiB, 250,000 products, and 1,000,000 bytes per field (including combined repeated XML values). CSV accepts up to 4,096 columns, 256 bytes per source header label, and 2,000,000 populated input cells. Corrected CSV is limited to 16,000,000 rectangular cells and 128 MiB. Inputs that exceed these limits produce an explicit error before a corrected file is published; the original file is preserved.
