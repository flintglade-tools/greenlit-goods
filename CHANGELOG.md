# Changelog

All notable changes are documented here. This project follows semantic versioning.

## [0.1.0] - 2026-07-04

### Added

- Offline RSS/XML and CSV/TSV feed auditing with human and schema-versioned JSON reports.
- Destination- and country-aware 2026 product-data checks with per-finding provenance.
- Exact-cent price parsing and per-currency revenue-at-risk estimates.
- Conservative enum, price, and boundary-whitespace fixes with a complete change log.
- Atomic no-clobber output and fail-closed XML rewrite eligibility.
- Resource limits, strict-mode CI exit codes, and adversarial parser/CLI coverage.

### Security

- Updated `quick-xml` and `anyhow` beyond the audited vulnerable versions identified during the pre-release review.
- Escaped untrusted control characters before terminal rendering.
