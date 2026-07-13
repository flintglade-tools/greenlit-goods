# Changelog

All notable changes are documented here. This project follows semantic versioning.

## [0.1.0] - 2026-07-13

### Added

- Offline RSS/XML and CSV/TSV feed auditing with human and schema-versioned JSON reports.
- Destination- and country-aware 2026 product-data checks with per-finding provenance.
- Exact-cent price parsing and per-currency revenue-at-risk estimates.
- Conservative enum, price, and boundary-whitespace fixes with a complete change log.
- Atomic no-clobber output and fail-closed XML rewrite eligibility.
- Resource limits, strict-mode CI exit codes, and adversarial parser/CLI coverage.
- Collision-free CSV header and surplus-column preservation.
- Architecture-labelled release archives for Linux x86_64, macOS arm64 and x86_64, and Windows x86_64.

### Security

- Updated `quick-xml` and `anyhow` beyond the audited vulnerable versions identified during the pre-release review.
- Escaped untrusted control characters before terminal rendering.
- Refused every XML rewrite outside the supported, lossless RSS/Atom grammar.
