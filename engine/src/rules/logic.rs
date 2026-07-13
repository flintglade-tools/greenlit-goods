//! The logic-heavy half of the rules engine: checks that need real computation
//! or cross-product context rather than a table lookup.

use crate::finding::{Finding, Severity};
use crate::model::Product;
use crate::report::Destination;
use crate::rules::declarative::normalize_enum;
use crate::rules::{FeedContext, Rule};
use crate::text::{has_html, is_shouty};

// ---------------------------------------------------------------------------
// Shared validation helpers
// ---------------------------------------------------------------------------

/// Validate a GTIN by length and GS1 check digit. Accepts GTIN-8/12/13/14.
/// Merchant Center requires ASCII digits only; formatting separators are not
/// silently removed because a submitted invalid GTIN is a disapproval risk.
pub(crate) fn gtin_is_valid(raw: &str) -> bool {
    if !raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let cleaned = raw
        .bytes()
        .map(|byte| u32::from(byte - b'0'))
        .collect::<Vec<_>>();
    let len = cleaned.len();
    if ![8usize, 12, 13, 14].contains(&len) {
        return false;
    }
    let check = cleaned[len - 1];
    // Weights 3,1,3,1,... applied from the rightmost payload digit.
    let mut sum = 0u32;
    for (i, &d) in cleaned[..len - 1].iter().rev().enumerate() {
        sum += d * if i % 2 == 0 { 3 } else { 1 };
    }
    let computed = (10 - (sum % 10)) % 10;
    computed == check
}

/// Is this a conservative, absolute HTTP(S) URL with a DNS host or bracketed
/// IPv6 literal? This is deliberately a syntax gate, not a reachability check.
fn is_http_url(s: &str) -> bool {
    let t = s.trim();
    if t.chars().any(|ch| ch.is_whitespace() || ch.is_control()) || !valid_percent_escapes(t) {
        return false;
    }
    let Some((scheme, rest)) = t.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return false;
    }

    if let Some(after_open) = authority.strip_prefix('[') {
        let Some((address, suffix)) = after_open.split_once(']') else {
            return false;
        };
        return address.parse::<std::net::Ipv6Addr>().is_ok()
            && (suffix.is_empty() || valid_port(suffix));
    }

    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    if port.is_some_and(|port| !valid_port(&format!(":{port}"))) {
        return false;
    }
    if !host.contains('.') || host.starts_with('.') || host.ends_with('.') {
        return false;
    }
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    })
}

fn valid_port(suffix: &str) -> bool {
    suffix
        .strip_prefix(':')
        .and_then(|port| port.parse::<u16>().ok())
        .is_some_and(|port| port > 0)
}

fn valid_percent_escapes(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if bytes
                .get(index + 1)
                .is_none_or(|byte| !byte.is_ascii_hexdigit())
                || bytes
                    .get(index + 2)
                    .is_none_or(|byte| !byte.is_ascii_hexdigit())
            {
                return false;
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    true
}

/// Current ISO 4217 alphabetic currency codes, verified against SIX
/// (the ISO maintenance agency) on 2026-07-04. Historical codes are excluded.
const CURRENCIES: &[&str] = &[
    "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT",
    "BHD", "BIF", "BMD", "BND", "BOB", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD", "CAD", "CDF",
    "CHF", "CLP", "CNY", "COP", "CRC", "CUP", "CVE", "CZK", "DJF", "DKK", "DOP", "DZD", "EGP",
    "ERN", "ETB", "EUR", "FJD", "FKP", "GBP", "GEL", "GHS", "GIP", "GMD", "GNF", "GTQ", "GYD",
    "HKD", "HNL", "HTG", "HUF", "IDR", "ILS", "INR", "IQD", "IRR", "ISK", "JMD", "JOD", "JPY",
    "KES", "KGS", "KHR", "KMF", "KPW", "KRW", "KWD", "KYD", "KZT", "LAK", "LBP", "LKR", "LRD",
    "LSL", "LYD", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT", "MOP", "MRU", "MUR", "MVR", "MWK",
    "MXN", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK", "NPR", "NZD", "OMR", "PAB", "PEN", "PGK",
    "PHP", "PKR", "PLN", "PYG", "QAR", "RON", "RSD", "RUB", "RWF", "SAR", "SBD", "SCR", "SDG",
    "SEK", "SGD", "SHP", "SLE", "SOS", "SRD", "SSP", "STN", "SVC", "SYP", "SZL", "THB", "TJS",
    "TMT", "TND", "TOP", "TRY", "TTD", "TWD", "TZS", "UAH", "UGX", "USD", "UYU", "UZS", "VED",
    "VES", "VND", "VUV", "WST", "XAF", "XCD", "XOF", "XPF", "YER", "ZAR", "ZMW", "ZWG",
];

struct PriceParse {
    amount_cents: Option<i64>,
    currency: Option<String>,
    currency_valid: bool,
    has_symbol: bool,
}

/// Parse a Merchant Center price string. The canonical form is
/// `"<amount> <ISO currency>"`, e.g. `"49.99 USD"`, but we tolerate and report on
/// the common deviations that remain unambiguous (symbol prefixes and missing
/// currency). Glued or reordered tokens are rejected.
fn parse_price(raw: &str) -> PriceParse {
    let mut t = raw.trim().to_string();
    let mut has_symbol = false;
    for sym in ['$', '£', '€', '¥'] {
        if t.starts_with(sym) {
            has_symbol = true;
            t = t.trim_start_matches(sym).trim().to_string();
            break;
        }
    }

    // Google price syntax is a numeric amount followed by an ISO currency.
    let tokens: Vec<&str> = t.split_whitespace().collect();
    let (num_str, cur_str): (String, Option<String>) = match tokens.as_slice() {
        [amount, currency] => (amount.to_string(), Some(currency.to_string())),
        [amount] => (amount.to_string(), None),
        _ => (String::new(), None),
    };

    let amount_cents = parse_amount_cents(&num_str);
    let currency = cur_str.map(|c| c.to_uppercase());
    let currency_valid = currency
        .as_deref()
        .map(|c| CURRENCIES.contains(&c))
        .unwrap_or(false);

    PriceParse {
        amount_cents,
        currency,
        currency_valid,
        has_symbol,
    }
}

/// Collect a product's GTIN from `gtin` or the common identifier synonyms.
fn gtin_value(p: &Product) -> Option<&str> {
    p.get("gtin")
        .or_else(|| p.get("upc"))
        .or_else(|| p.get("ean"))
        .or_else(|| p.get("isbn"))
        .or_else(|| p.get("jan"))
}

/// The parsed numeric amount and currency of a price, for scoring/reporting.
/// Unlike [`canonical_price`], this returns the amount even when the currency is
/// missing or unrecognized, so revenue estimates still have a number to work
/// with.
pub(crate) fn price_parts(raw: &str) -> (Option<i64>, Option<String>) {
    let p = parse_price(raw);
    let currency = p.currency.filter(|_| p.currency_valid);
    (p.amount_cents, currency)
}

/// The canonical, normalized form of a price — `"49.90 USD"` — or `None` if it
/// can't be produced *safely*. Returns `None` for symbol-prefixed prices (the
/// currency is ambiguous), missing/invalid currencies (we never invent one), and
/// non-positive amounts. Shared with the fixer so normalization and validation
/// agree.
pub(crate) fn canonical_price(raw: &str) -> Option<String> {
    let parsed = parse_price(raw);
    if parsed.has_symbol {
        return None;
    }
    let amount_cents = parsed.amount_cents?;
    if amount_cents <= 0 {
        return None;
    }
    let cur = parsed.currency?;
    if !parsed.currency_valid {
        return None;
    }
    Some(format_cents(amount_cents, &cur))
}

fn parse_amount_cents(raw: &str) -> Option<i64> {
    if raw.is_empty() || raw.starts_with(['+', '-']) {
        return None;
    }
    let mut parts = raw.split('.');
    let whole = parts.next()?;
    let fraction = parts.next();
    if parts.next().is_some() {
        return None;
    }
    let groups: Vec<&str> = whole.split(',').collect();
    let whole_digits = if groups.len() == 1 {
        let group = groups[0];
        if group.is_empty() || !group.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        group.to_string()
    } else {
        if groups[0].is_empty()
            || groups[0].len() > 3
            || !groups[0].bytes().all(|byte| byte.is_ascii_digit())
            || groups[1..]
                .iter()
                .any(|group| group.len() != 3 || !group.bytes().all(|byte| byte.is_ascii_digit()))
        {
            return None;
        }
        groups.concat()
    };
    let fraction_cents = match fraction {
        None => 0,
        Some(value) if value.len() == 1 && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value.parse::<i64>().ok()?.checked_mul(10)?
        }
        Some(value) if value.len() == 2 && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value.parse::<i64>().ok()?
        }
        _ => return None,
    };
    whole_digits
        .parse::<i64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(fraction_cents)
}

fn format_cents(cents: i64, currency: &str) -> String {
    format!("{}.{:02} {currency}", cents / 100, cents % 100)
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

pub struct GtinRule;
impl Rule for GtinRule {
    fn id(&self) -> &'static str {
        "gtin"
    }
    fn check(&self, p: &Product, _ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        if let Some(g) = gtin_value(p) {
            if !gtin_is_valid(g) {
                out.push(
                    Finding::new(
                        "GL-GTIN-INVALID",
                        Severity::Disapproval,
                        format!("GTIN \"{g}\" is not a valid 8/12/13/14-digit barcode."),
                    )
                    .field("gtin")
                    .detail(
                        "Fails the GS1 length/check-digit test. An invalid GTIN is worse than \
                         none — set identifier_exists to no if the product truly has none.",
                    ),
                );
            }
        }
        out
    }
}

pub struct IdentifierRule;
impl Rule for IdentifierRule {
    fn id(&self) -> &'static str {
        "identifier"
    }
    fn check(&self, p: &Product, _ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        let has_valid_gtin = gtin_value(p).map(gtin_is_valid).unwrap_or(false);
        let has_brand = p.get("brand").is_some();
        let has_mpn = p.get("mpn").is_some();
        let id_exists_no = p
            .get("identifier_exists")
            .and_then(|v| normalize_enum("identifier_exists", v))
            == Some("no");

        if !id_exists_no {
            if !(has_valid_gtin || has_brand && has_mpn) {
                out.push(
                    Finding::new(
                        "GL-IDENTIFIER",
                        Severity::AtRisk,
                        "No valid GTIN and no brand+MPN pair; product lacks a unique identifier.",
                    )
                    .detail(
                        "Provide a GTIN, or brand together with MPN. If the product genuinely \
                         has no identifier, set identifier_exists to no.",
                    ),
                );
            }
            if !has_brand {
                out.push(
                    Finding::new(
                        "GL-REQ-brand",
                        Severity::AtRisk,
                        "Brand is missing; it is required for most products.",
                    )
                    .field("brand")
                    .detail("Exceptions are limited (e.g. some media, custom goods)."),
                );
            }
        }
        out
    }
}

pub struct UrlRule;
impl Rule for UrlRule {
    fn id(&self) -> &'static str {
        "url"
    }
    fn check(&self, p: &Product, _ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        for (attr, severity) in [
            ("link", Severity::Disapproval),
            ("image_link", Severity::Disapproval),
            ("mobile_link", Severity::AtRisk),
            ("video_link", Severity::AtRisk),
        ] {
            if let Some(v) = p.get(attr) {
                if !is_http_url(v) {
                    out.push(
                        Finding::new(
                            format!("GL-URL-{attr}"),
                            severity,
                            format!("'{attr}' is not a valid absolute http(s) URL."),
                        )
                        .field(attr)
                        .detail("Must start with http:// or https:// and contain no spaces."),
                    );
                }
            }
        }
        // additional_image_link may hold several comma-joined URLs.
        if let Some(v) = p.get("additional_image_link") {
            for url in v.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if !is_http_url(url) {
                    out.push(
                        Finding::new(
                            "GL-URL-additional_image_link",
                            Severity::AtRisk,
                            format!("An additional image URL is invalid: \"{url}\"."),
                        )
                        .field("additional_image_link"),
                    );
                    break;
                }
            }
        }
        out
    }
}

pub struct PriceRule;
impl Rule for PriceRule {
    fn id(&self) -> &'static str {
        "price"
    }
    fn check(&self, p: &Product, _ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        if let Some(raw) = p.get("price") {
            let parsed = parse_price(raw);
            match parsed.amount_cents {
                None => out.push(
                    Finding::new(
                        "GL-PRICE-FORMAT",
                        Severity::Disapproval,
                        format!("Price \"{raw}\" is not a parseable amount."),
                    )
                    .field("price")
                    .detail("Use the form \"49.99 USD\"."),
                ),
                Some(a) if a <= 0 => out.push(
                    Finding::new(
                        "GL-PRICE-ZERO",
                        Severity::Disapproval,
                        "Price is zero or negative.",
                    )
                    .field("price"),
                ),
                Some(_) => {
                    if parsed.has_symbol {
                        out.push(
                            Finding::new(
                                "GL-PRICE-SYMBOL",
                                Severity::AtRisk,
                                format!("Price \"{raw}\" uses a currency symbol; use an ISO code."),
                            )
                            .field("price")
                            .detail("Symbols are ambiguous ($ could be USD, CAD, AUD…); not auto-fixed."),
                        );
                    } else if parsed.currency.is_none() {
                        out.push(
                            Finding::new(
                                "GL-PRICE-NOCUR",
                                Severity::AtRisk,
                                "Price has no currency code.",
                            )
                            .field("price")
                            .detail("Google may infer it from the account, but stating it is safer; not auto-filled."),
                        );
                    } else if !parsed.currency_valid {
                        out.push(
                            Finding::new(
                                "GL-PRICE-CUR",
                                Severity::Disapproval,
                                format!(
                                    "Currency \"{}\" is not a recognized ISO 4217 code.",
                                    parsed.currency.as_deref().unwrap_or("")
                                ),
                            )
                            .field("price"),
                        );
                    }
                }
            }
        }
        // sale_price sanity vs price.
        if let (Some(price_raw), Some(sale_raw)) = (p.get("price"), p.get("sale_price")) {
            let regular = parse_price(price_raw);
            let sale_value = parse_price(sale_raw);
            let pp = regular.amount_cents;
            let sp = sale_value.amount_cents;
            if let (Some(price), Some(sale)) = (pp, sp) {
                if sale > price {
                    out.push(
                        Finding::new(
                            "GL-SALE-HIGH",
                            Severity::AtRisk,
                            "sale_price is greater than price.",
                        )
                        .field("sale_price")
                        .detail("A sale price above the regular price will be ignored or flagged."),
                    );
                }
            }
            if regular.currency_valid
                && sale_value.currency_valid
                && regular.currency != sale_value.currency
            {
                out.push(
                    Finding::new(
                        "GL-SALE-CURRENCY",
                        Severity::Disapproval,
                        "sale_price currency does not match price currency.",
                    )
                    .field("sale_price"),
                );
            }
        }
        out
    }
}

pub struct PreorderDateRule;
impl Rule for PreorderDateRule {
    fn id(&self) -> &'static str {
        "preorder_date"
    }
    fn check(&self, p: &Product, _ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        if let Some(av) = p.get("availability") {
            let norm = normalize_enum("availability", av).unwrap_or("");
            if (norm == "preorder" || norm == "backorder") && p.get("availability_date").is_none() {
                out.push(
                    Finding::new(
                        "GL-PREORDER-DATE",
                        Severity::Disapproval,
                        format!("availability is '{norm}' but availability_date is missing."),
                    )
                    .field("availability_date")
                    .detail(
                        "Google requires an availability date for preorder/backorder products.",
                    ),
                );
            }
        }
        out
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ApparelMatch {
    Apparel,
    ClothingOrShoes,
    Heuristic,
    No,
}

fn apparel_match(p: &Product) -> ApparelMatch {
    if let Some(cat) = p.get("google_product_category") {
        let c = cat.to_lowercase();
        let category_id = c
            .split(|ch: char| !ch.is_ascii_digit())
            .next()
            .and_then(|id| id.parse::<u32>().ok());
        if matches!(category_id, Some(1604 | 187))
            || c.contains("apparel & accessories > clothing")
            || c.contains("apparel & accessories > shoes")
        {
            return ApparelMatch::ClothingOrShoes;
        }
        if category_id == Some(166) || c.contains("apparel & accessories") {
            return ApparelMatch::Apparel;
        }
    }
    if let Some(pt) = p.get("product_type") {
        let t = pt.to_lowercase();
        const KW: &[&str] = &[
            "apparel", "clothing", "shirt", "dress", "shoe", "footwear", "pants", "jacket",
            "jeans", "hoodie", "sweater", "skirt", "coat",
        ];
        if KW.iter().any(|k| t.contains(k)) {
            return ApparelMatch::Heuristic;
        }
    }
    ApparelMatch::No
}

pub struct ApparelRule;
impl Rule for ApparelRule {
    fn id(&self) -> &'static str {
        "apparel"
    }
    fn check(&self, p: &Product, ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        let matched = apparel_match(p);
        if matched == ApparelMatch::No {
            return out;
        }

        let strict_market = ["BR", "FR", "DE", "JP", "GB", "US"]
            .contains(&ctx.target_country.to_ascii_uppercase().as_str());
        let normative_required = matched != ApparelMatch::Heuristic
            && (ctx.destination == Destination::FreeListings || strict_market);
        if !normative_required && matched != ApparelMatch::Heuristic {
            return out;
        }
        let severity = if normative_required {
            Severity::Disapproval
        } else {
            Severity::AtRisk
        };
        let mut required = vec!["color", "gender", "age_group"];
        if matched == ApparelMatch::ClothingOrShoes || matched == ApparelMatch::Heuristic {
            required.push("size");
        }
        for attr in required {
            if p.get(attr).is_none() {
                let finding = Finding::new(
                        format!("GL-APPAREL-{attr}"),
                        severity,
                        format!("Apparel product is missing '{attr}'."),
                    )
                    .field(attr)
                    .detail(
                        if matched == ApparelMatch::Heuristic {
                            "Heuristic product_type match only; confirm the Google category before treating this as required."
                        } else {
                            "Requirement derived from Google category, destination, and target-country rules verified 2026-07-04."
                        },
                    );
                out.push(if matched == ApparelMatch::Heuristic {
                    finding.heuristic()
                } else {
                    finding
                });
            }
        }
        out
    }
}

pub struct VariantRule;
impl Rule for VariantRule {
    fn id(&self) -> &'static str {
        "variant"
    }
    fn check(&self, p: &Product, ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        if let Some(gid) = p.get("item_group_id") {
            let shared = ctx.group_counts.get(gid).copied().unwrap_or(0) > 1;
            if shared {
                const DISTINGUISHING: &[&str] = &[
                    "color",
                    "size",
                    "pattern",
                    "material",
                    "gender",
                    "age_group",
                    "size_type",
                ];
                if !DISTINGUISHING.iter().any(|a| p.get(a).is_some()) {
                    out.push(
                        Finding::new(
                            "GL-VARIANT",
                            Severity::AtRisk,
                            "Variant shares item_group_id but has no distinguishing attribute.",
                        )
                        .field("item_group_id")
                        .detail("Variants in a group must differ by color, size, pattern, etc."),
                    );
                }
            }
        }
        out
    }
}

pub struct DuplicateIdRule;
impl Rule for DuplicateIdRule {
    fn id(&self) -> &'static str {
        "duplicate_id"
    }
    fn check(&self, p: &Product, ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        if let Some(id) = p.get("id") {
            if ctx.id_counts.get(id).copied().unwrap_or(0) > 1 {
                out.push(
                    Finding::new(
                        "GL-DUP-ID",
                        Severity::Disapproval,
                        format!("Duplicate product id \"{id}\" appears more than once."),
                    )
                    .field("id")
                    .detail("Every product needs a unique id; duplicates collide and disapprove."),
                );
            }
        }
        out
    }
}

pub struct TitleQualityRule;
impl Rule for TitleQualityRule {
    fn id(&self) -> &'static str {
        "title_quality"
    }
    fn check(&self, p: &Product, _ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        if let Some(title) = p.get("title") {
            if has_html(title) {
                out.push(
                    Finding::new(
                        "GL-TITLE-HTML",
                        Severity::Optimization,
                        "Title contains HTML.",
                    )
                    .field("title"),
                );
            }
            if is_shouty(title) {
                out.push(
                    Finding::new(
                        "GL-TITLE-CAPS",
                        Severity::Optimization,
                        "Title is in all caps.",
                    )
                    .field("title")
                    .detail("ALL-CAPS titles are discouraged; casing changes require review."),
                );
            }
            let lt = title.to_lowercase();
            const PROMO: &[&str] = &[
                "free shipping",
                "best price",
                "lowest price",
                "buy now",
                "% off",
                "clearance",
                "!!!",
                "$$$",
            ];
            if PROMO.iter().any(|k| lt.contains(k)) {
                out.push(
                    Finding::new(
                        "GL-TITLE-PROMO",
                        Severity::AtRisk,
                        "Title contains promotional text or excessive punctuation.",
                    )
                    .field("title")
                    .detail("Google disallows promotional phrases in titles; left for human edit."),
                );
            }
            if title.chars().count() < 10 {
                out.push(
                    Finding::new(
                        "GL-TITLE-SHORT",
                        Severity::Optimization,
                        "Title is very short (under 10 characters).",
                    )
                    .field("title")
                    .heuristic(),
                );
            }
        }
        out
    }
}

pub struct DescriptionQualityRule;
impl Rule for DescriptionQualityRule {
    fn id(&self) -> &'static str {
        "description_quality"
    }
    fn check(&self, p: &Product, _ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        if let Some(d) = p.get("description") {
            if has_html(d) {
                out.push(
                    Finding::new(
                        "GL-DESC-HTML",
                        Severity::Optimization,
                        "Description contains HTML markup.",
                    )
                    .field("description"),
                );
            }
            if d.chars().count() < 30 {
                out.push(
                    Finding::new(
                        "GL-DESC-SHORT",
                        Severity::Optimization,
                        "Description is very short (under 30 characters).",
                    )
                    .field("description")
                    .heuristic(),
                );
            }
        }
        out
    }
}

pub struct ImagePlaceholderRule;
impl Rule for ImagePlaceholderRule {
    fn id(&self) -> &'static str {
        "image_placeholder"
    }
    fn check(&self, p: &Product, _ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        if let Some(img) = p.get("image_link") {
            let l = img.to_lowercase();
            const BAD: &[&str] = &[
                "example.com",
                "placeholder",
                "no-image",
                "noimage",
                "default.jpg",
                "dummy",
            ];
            if BAD.iter().any(|k| l.contains(k)) {
                out.push(
                    Finding::new(
                        "GL-IMG-PLACEHOLDER",
                        Severity::AtRisk,
                        "Image URL looks like a placeholder.",
                    )
                    .field("image_link")
                    .detail("Heuristic match; verify a real product image is served.")
                    .heuristic(),
                );
            }
        }
        out
    }
}

pub struct WhitespaceRule;
impl Rule for WhitespaceRule {
    fn id(&self) -> &'static str {
        "whitespace"
    }
    fn check(&self, p: &Product, _ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        for attr in ["title", "description", "brand"] {
            if p.get(attr).is_some() {
                if let Some(raw) = p.get_raw(attr) {
                    if raw != raw.trim() {
                        out.push(
                            Finding::new(
                                "GL-WS-TRIM",
                                Severity::Optimization,
                                format!("'{attr}' has leading or trailing whitespace."),
                            )
                            .field(attr)
                            .fixable()
                            .heuristic(),
                        );
                    }
                }
            }
        }
        if let Some(title) = p.get("title") {
            if title.contains("  ") {
                out.push(
                    Finding::new(
                        "GL-WS-DOUBLE",
                        Severity::Optimization,
                        "Title contains repeated spaces.",
                    )
                    .field("title")
                    .heuristic(),
                );
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gtin_checksum_validates_real_barcodes() {
        // Verified GS1 check digits across all four GTIN lengths.
        assert!(gtin_is_valid("73513537")); // GTIN-8
        assert!(gtin_is_valid("012345678905")); // GTIN-12 (UPC-A)
        assert!(gtin_is_valid("4006381333931")); // GTIN-13 (EAN-13)
        assert!(gtin_is_valid("0712345678904")); // GTIN-13 leading zero
    }

    #[test]
    fn gtin_rejects_bad_input() {
        assert!(!gtin_is_valid("73513538")); // wrong check digit
        assert!(!gtin_is_valid("1234567890")); // 10 digits: invalid length
        assert!(!gtin_is_valid("12345")); // too short
        assert!(!gtin_is_valid("ABC0381333931")); // non-digit
        assert!(!gtin_is_valid("4006381-333931")); // separators are not allowed
        assert!(!gtin_is_valid("")); // empty
    }

    #[test]
    fn price_canonicalization_is_safe_only() {
        assert_eq!(canonical_price("49.99 USD").as_deref(), Some("49.99 USD"));
        assert_eq!(canonical_price("19.9 usd").as_deref(), Some("19.90 USD"));
        assert_eq!(
            canonical_price("1,299.00 EUR").as_deref(),
            Some("1299.00 EUR")
        );
        // Never invents currency from a symbol.
        assert_eq!(canonical_price("$29.99"), None);
        // Never fixes a missing currency.
        assert_eq!(canonical_price("29.99"), None);
        // Non-positive and unrecognized currency are not "fixed".
        assert_eq!(canonical_price("0 USD"), None);
        assert_eq!(canonical_price("10.00 XYZ"), None);
        assert_eq!(canonical_price("free"), None);
        assert_eq!(canonical_price("1e309 USD"), None);
        assert_eq!(canonical_price("1,2,3.00 USD"), None);
        assert_eq!(canonical_price("10.001 USD"), None);
        assert_eq!(canonical_price("USD 10.00"), None);
        assert_eq!(canonical_price("10.00 RON"), Some("10.00 RON".into()));
        assert_eq!(canonical_price("10.00 VND"), Some("10.00 VND".into()));
        assert_eq!(canonical_price("10.00 BGN"), None);
    }

    #[test]
    fn price_parts_extracts_amount_and_valid_currency() {
        assert_eq!(
            price_parts("49.99 USD"),
            (Some(4999), Some("USD".to_string()))
        );
        // Symbol stripped, amount kept, but no ISO currency reported.
        assert_eq!(price_parts("$29.99"), (Some(2999), None));
        // Glued tokens are not valid Google price syntax.
        assert_eq!(price_parts("15.00EUR"), (None, None));
        // Invalid currency is dropped (amount still parsed).
        assert_eq!(price_parts("10.00 XYZ"), (Some(1000), None));
    }

    #[test]
    fn url_validation() {
        assert!(is_http_url("https://example.com/x.jpg"));
        assert!(is_http_url("http://a.co"));
        assert!(!is_http_url("ftp://example.com"));
        assert!(!is_http_url("/relative/path"));
        assert!(!is_http_url("https://no spaces.com"));
        assert!(!is_http_url("https://example..com"));
        assert!(!is_http_url("https://example.com:abc/path"));
        assert!(!is_http_url("https://user@example.com/path"));
        assert!(!is_http_url("https://example.com/%ZZ"));
        assert!(is_http_url("https://example.com:443/a%20b"));
        assert!(is_http_url("https://[2001:db8::1]/image.jpg"));
        assert!(!is_http_url("not-a-url"));
    }
}
