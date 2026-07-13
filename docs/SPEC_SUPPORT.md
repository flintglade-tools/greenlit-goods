# Product specification support

Last full verification: 2026-07-04

Structured title and structured description requirements rechecked: 2026-07-13

Primary sources:

- Product data specification: https://support.google.com/merchants/answer/7052112?hl=en
- 2026 product-data changes: https://support.google.com/merchants/answer/16989427
- RSS 2.0 feed format: https://support.google.com/merchants/answer/14987622?hl=en
- ISO 4217 maintenance agency: https://www.six-group.com/en/products-services/financial-information/market-reference-data/data-standards.html

## Supported in v0.1

| Area | Coverage | Boundary |
|---|---|---|
| Required attributes | `id`, title or structured title, description or structured description, link, image, availability, price | Feed-local presence checks; length checks cover plain title/description, while grouped alternatives are presence-only until their sub-attributes are parsed |
| Enumerations | condition, availability, gender, age group, size type, adult, bundle, identifier flags | Known documented values and safe canonical variants |
| Prices | Exact non-negative amount with at most two decimals and a current ISO 4217 alphabetic code | No symbols, historical codes, currency-first forms, exponents, inferred currency, market/country currency matching, tax, or exchange rates |
| Identifiers | GTIN-8/12/13/14 checksum, brand/MPN combination, identifier opt-out | Does not query registries or validate brand ownership |
| Apparel | Country/destination conditions; size, color, gender, age-group checks for known apparel categories | Keyword fallback is explicitly heuristic and cannot create a disapproval |
| Target country | Current ISO 3166-1 alpha-2 codes; case-insensitive input | Google destination availability is not queried |
| Availability dates | Preorder/backorder relationship | Syntax/field relationship only |
| URLs | Basic HTTP(S) syntax and selected image/video fields | No fetch, redirect, ownership, content, or TLS validation |
| Feed structure | RSS Google namespace, malformed/truncated XML, ragged CSV, duplicate headers | Unsupported XML remains auditable where possible but is never rewritten |
| Listing quality | Short/shouty/promotional text and placeholder-image signals | Heuristic/optimization findings only where enforcement is not established |

## Deliberately excluded

- Account, policy, suspension, destination eligibility, diagnostics API, and historical serving state.
- Landing-page price/availability parity, structured-data parity, robots behavior, redirects, and crawl results.
- Image dimensions, encoding, content, watermarks, generated-image metadata, or computer-vision policy checks.
- Full shipping, tax, loyalty, promotion, subscription, installment, certification, energy-label, vehicle, and regional-inventory models.
- Category taxonomy validation beyond the narrow apparel classifications used by conditional rules.

## Provenance rules

Normative findings use `basis: google_specification` and include the source URL and `source_verified_on`. Greenlit-created signals use `basis: greenlit_heuristic` without implying Google enforcement. Parser/rewrite findings use `basis: feed_structure`.

Specification pages change independently of this repository. A release is blocked until maintainers re-check the primary sources, update this matrix, and run the full test suite.
