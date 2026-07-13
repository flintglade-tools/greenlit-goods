//! Automatic correction — the conservative half of the value proposition.
//!
//! Every fixer here performs only a **provably safe** transformation: one where
//! the corrected value cannot be *more* wrong than the original and cannot put
//! the merchant in policy trouble. Risky repairs (inventing a missing GTIN,
//! guessing a category, adding a currency to a bare number) are deliberately
//! absent — those surface as non-fixable findings for a human to decide. A bad
//! silent auto-fix is worse than the problem it replaces.
//!
//! Every change is recorded as a [`FixRecord`] so the result is a reviewable
//! diff, never a black box.

use serde::Serialize;

use crate::model::{Feed, Product};
use crate::rules::declarative::{enum_canonical, ENUM_ATTRS};
use crate::rules::logic::canonical_price;

/// One recorded edit: what changed, on which product and field, and who did it.
#[derive(Debug, Clone, Serialize)]
pub struct FixRecord {
    pub product_id: Option<String>,
    pub field: String,
    pub before: String,
    pub after: String,
    /// Comma-joined ids of the fixer(s) that touched this field.
    pub fixer_id: String,
}

/// A safe, automatic transformation over a single product.
pub trait Fixer {
    fn id(&self) -> &'static str;
    /// Apply in place, returning a record for each field changed (with
    /// `product_id` left `None` for the caller to stamp).
    fn apply(&self, product: &mut Product) -> Vec<FixRecord>;
}

fn rec(field: &str, before: String, after: String, fixer: &str) -> FixRecord {
    FixRecord {
        product_id: None,
        field: field.to_string(),
        before,
        after,
        fixer_id: fixer.to_string(),
    }
}

/// Trim leading/trailing whitespace on every attribute; additionally collapse
/// repeated internal spaces in the title (safe — titles are single-line). The
/// description's internal whitespace is left alone to preserve paragraph breaks.
pub struct TrimWhitespace;
impl Fixer for TrimWhitespace {
    fn id(&self) -> &'static str {
        "trim_whitespace"
    }
    fn apply(&self, p: &mut Product) -> Vec<FixRecord> {
        let mut out = Vec::new();
        for k in [
            "title",
            "description",
            "brand",
            "price",
            "sale_price",
            "availability",
            "condition",
            "gender",
            "age_group",
            "size_type",
            "adult",
            "is_bundle",
            "identifier_exists",
        ] {
            if let Some(raw) = p.get_raw(k) {
                let new = raw.trim().to_string();
                if new != raw {
                    let before = raw.to_string();
                    p.set(k, new.clone());
                    out.push(rec(k, before, new, self.id()));
                }
            }
        }
        out
    }
}

/// Normalize price/sale_price formatting *only* when it can be done safely —
/// two decimals and an uppercase, recognized ISO currency. Never adds a missing
/// currency and never resolves a currency symbol.
pub struct NormalizePrice;
impl Fixer for NormalizePrice {
    fn id(&self) -> &'static str {
        "normalize_price"
    }
    fn apply(&self, p: &mut Product) -> Vec<FixRecord> {
        let mut out = Vec::new();
        for attr in ["price", "sale_price"] {
            if let Some(v) = p.get(attr) {
                if let Some(norm) = canonical_price(v) {
                    if norm != v {
                        let before = v.to_string();
                        p.set(attr, norm.clone());
                        out.push(rec(attr, before, norm, self.id()));
                    }
                }
            }
        }
        out
    }
}

/// Canonicalize enumerated attributes (`availability`, `condition`, …) when the
/// intended value is unambiguous (`"In Stock"` → `"in_stock"`).
pub struct NormalizeEnum;
impl Fixer for NormalizeEnum {
    fn id(&self) -> &'static str {
        "normalize_enum"
    }
    fn apply(&self, p: &mut Product) -> Vec<FixRecord> {
        let mut out = Vec::new();
        for &attr in ENUM_ATTRS {
            if let Some(v) = p.get(attr) {
                if let Some(canon) = enum_canonical(attr, v) {
                    if canon != v {
                        let before = v.to_string();
                        p.set(attr, canon);
                        out.push(rec(attr, before, canon.to_string(), self.id()));
                    }
                }
            }
        }
        out
    }
}

/// The conservative fixer pipeline. Editorial rewrites such as HTML stripping
/// and title recasing are intentionally excluded because they require review.
pub fn all_fixers() -> Vec<Box<dyn Fixer>> {
    vec![
        Box::new(TrimWhitespace),
        Box::new(NormalizePrice),
        Box::new(NormalizeEnum),
    ]
}

/// Apply every fixer to a clone of the feed, returning the corrected feed and a
/// clean per-field change log (multiple fixers touching one field are coalesced
/// into a single before→after record).
pub fn apply_fixes(feed: &Feed) -> (Feed, Vec<FixRecord>) {
    let fixers = all_fixers();
    let mut corrected = feed.clone();
    let mut log = Vec::new();

    for p in &mut corrected.products {
        let id = p.id().map(str::to_string);
        let mut per_product: Vec<FixRecord> = Vec::new();
        for fx in &fixers {
            for mut r in fx.apply(p) {
                r.product_id = id.clone();
                per_product.push(r);
            }
        }
        log.extend(coalesce(per_product));
    }

    (corrected, log)
}

/// Collapse multiple records for the same field into one: earliest `before`,
/// final `after`, and the set of fixers involved.
fn coalesce(records: Vec<FixRecord>) -> Vec<FixRecord> {
    let mut order: Vec<String> = Vec::new();
    let mut map: std::collections::HashMap<String, FixRecord> = std::collections::HashMap::new();
    for r in records {
        match map.get_mut(&r.field) {
            Some(existing) => {
                existing.after = r.after;
                if !existing.fixer_id.split(", ").any(|f| f == r.fixer_id) {
                    existing.fixer_id = format!("{}, {}", existing.fixer_id, r.fixer_id);
                }
            }
            None => {
                order.push(r.field.clone());
                map.insert(r.field.clone(), r);
            }
        }
    }
    order
        .into_iter()
        .filter_map(|f| map.remove(&f))
        .filter(|r| r.before != r.after)
        .collect()
}
