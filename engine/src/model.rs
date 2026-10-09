//! The normalized in-memory model that every input format parses into.
//!
//! The central design constraint is preservation within the explicitly
//! supported rewrite subset: a product may carry attributes the engine has
//! never heard of, and corrupting or dropping them on output would be worse than
//! any value we add. A [`Product`] is therefore an insertion-ordered attribute
//! map, not a rigid struct. Unsupported XML structure makes rewriting fail
//! closed; represented scalar values remain verbatim until a logged fixer acts.

use serde::Serialize;
use std::collections::HashMap;

use crate::finding::Finding;

/// Supported input/output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// RSS 2.0 with the Google base namespace (`g:` prefixed attributes). This is
    /// the canonical Merchant Center XML feed shape. Atom is tolerated on input.
    Xml,
    /// Delimited text — comma or tab separated, with a header row.
    Csv,
}

/// An insertion-ordered, case-normalized attribute bag for one product.
///
/// Lookups use canonical attribute names (lowercase, underscore-separated, `g:`
/// stripped). Insertion order is preserved so we can write the feed back out in
/// a stable, human-recognizable shape.
#[derive(Debug, Clone, Default)]
pub struct Product {
    keys: Vec<String>,
    values: Vec<String>,
    index: HashMap<String, usize>,
    /// 1-based source line/record number, for diagnostics. Best-effort.
    pub source_record: Option<u64>,
}

impl Product {
    pub fn new() -> Self {
        Product::default()
    }

    /// Insert or overwrite an attribute. The key is canonicalized; the original
    /// value is stored verbatim. Returns the previous value if any.
    pub fn set(&mut self, attr: impl AsRef<str>, value: impl Into<String>) -> Option<String> {
        let key = canonical_attr(attr.as_ref());
        let value = value.into();
        if let Some(&pos) = self.index.get(&key) {
            Some(std::mem::replace(&mut self.values[pos], value))
        } else {
            let pos = self.keys.len();
            self.index.insert(key.clone(), pos);
            self.keys.push(key);
            self.values.push(value);
            None
        }
    }

    /// Like [`set`](Self::set) but only if the attribute is absent — used by
    /// parsers so the first occurrence of a repeated element wins for scalars.
    pub fn set_if_absent(&mut self, attr: impl AsRef<str>, value: impl Into<String>) {
        let key = canonical_attr(attr.as_ref());
        if !self.index.contains_key(&key) {
            self.set(key, value);
        }
    }

    /// Get a trimmed view of an attribute value. Returns `None` if the attribute
    /// is absent **or** present-but-empty/whitespace — for validation purposes a
    /// blank attribute is the same as a missing one.
    pub fn get(&self, attr: &str) -> Option<&str> {
        let key = canonical_attr(attr);
        let pos = *self.index.get(&key)?;
        let v = self.values[pos].trim();
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    }

    /// Get the raw, untrimmed stored value (present even if blank).
    pub fn get_raw(&self, attr: &str) -> Option<&str> {
        let key = canonical_attr(attr);
        let pos = *self.index.get(&key)?;
        Some(&self.values[pos])
    }

    pub(crate) fn get_raw_mut(&mut self, attr: &str) -> Option<&mut String> {
        let pos = *self.index.get(&canonical_attr(attr))?;
        Some(&mut self.values[pos])
    }

    /// True if the attribute exists at all, even if blank.
    pub fn has(&self, attr: &str) -> bool {
        self.index.contains_key(&canonical_attr(attr))
    }

    /// Remove an attribute, returning its value if present.
    pub fn remove(&mut self, attr: &str) -> Option<String> {
        let key = canonical_attr(attr);
        let pos = self.index.remove(&key)?;
        self.keys.remove(pos);
        let val = self.values.remove(pos);
        // Re-index everything after the removed position.
        for (_, p) in self.index.iter_mut() {
            if *p > pos {
                *p -= 1;
            }
        }
        Some(val)
    }

    /// The product's `id`, the natural primary key.
    pub fn id(&self) -> Option<&str> {
        self.get("id")
    }

    /// Iterate `(canonical_attr, value)` in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.keys
            .iter()
            .zip(self.values.iter())
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub fn attributes(&self) -> &[String] {
        &self.keys
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// RSS channel-level metadata, preserved so XML round-trips keep its envelope.
#[derive(Debug, Clone, Default)]
pub struct ChannelMeta {
    pub title: Option<String>,
    pub link: Option<String>,
    pub description: Option<String>,
}

/// A parsed feed: the products, the channel envelope (XML), and any structural
/// parse findings that arose before rules ever ran (e.g. a ragged CSV row).
#[derive(Debug, Clone)]
pub struct Feed {
    pub format: Format,
    pub products: Vec<Product>,
    pub channel: ChannelMeta,
    /// Whether the delimited input used a tab (vs comma). Irrelevant for XML.
    pub csv_delimiter: u8,
    /// Canonical internal column keys in the source header order.
    pub(crate) csv_columns: Vec<String>,
    /// Original header labels keyed by their disambiguated internal key.
    pub(crate) csv_header_labels: HashMap<String, String>,
    /// Structural problems detected during parsing, surfaced alongside rule
    /// findings in the final report.
    pub parse_findings: Vec<Finding>,
    /// False when parsing recovered only a partial representation of the input.
    /// Audits can still explain the damage, but the feed must not be scored or
    /// rewritten as though it were complete.
    pub structurally_complete: bool,
    /// Reasons a rewrite would not preserve the input's product-data semantics.
    /// Empty means the parser proved the supported representation is rewrite-safe.
    pub rewrite_blockers: Vec<String>,
}

impl Feed {
    pub fn new(format: Format) -> Self {
        Feed {
            format,
            products: Vec::new(),
            channel: ChannelMeta::default(),
            csv_delimiter: b',',
            csv_columns: Vec::new(),
            csv_header_labels: HashMap::new(),
            parse_findings: Vec::new(),
            structurally_complete: true,
            rewrite_blockers: Vec::new(),
        }
    }

    pub fn block_rewrite(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if !self.rewrite_blockers.contains(&reason) {
            self.rewrite_blockers.push(reason);
        }
    }

    pub fn mark_incomplete(&mut self, reason: impl Into<String>) {
        self.structurally_complete = false;
        self.block_rewrite(reason);
    }

    pub fn rewrite_safe(&self) -> bool {
        self.structurally_complete && self.rewrite_blockers.is_empty()
    }

    /// The ordered union of every attribute seen across all products — the
    /// column set for CSV output. Order: by first appearance, but with the core
    /// Google attributes hoisted to the front in canonical order for readability.
    pub fn column_union(&self) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        let mut set: HashMap<String, ()> = HashMap::new();
        if self.format == Format::Csv {
            for column in &self.csv_columns {
                if set.insert(column.clone(), ()).is_none() {
                    seen.push(column.clone());
                }
            }
            for product in &self.products {
                for (key, _) in product.iter() {
                    if set.insert(key.to_string(), ()).is_none() {
                        seen.push(key.to_string());
                    }
                }
            }
            return seen;
        }
        // First pass: hoist known core columns in canonical order if present.
        for core in CORE_OUTPUT_ORDER {
            if self.products.iter().any(|p| p.has(core))
                && set.insert((*core).to_string(), ()).is_none()
            {
                seen.push((*core).to_string());
            }
        }
        // Second pass: everything else by first appearance.
        for p in &self.products {
            for (k, _) in p.iter() {
                if set.insert(k.to_string(), ()).is_none() {
                    seen.push(k.to_string());
                }
            }
        }
        seen
    }
}

/// Canonical ordering for the best-known Google attributes when emitting CSV or
/// ordering XML elements. Not exhaustive — unknown attributes follow in
/// first-seen order.
pub const CORE_OUTPUT_ORDER: &[&str] = &[
    "id",
    "title",
    "description",
    "link",
    "image_link",
    "additional_image_link",
    "mobile_link",
    "availability",
    "availability_date",
    "price",
    "sale_price",
    "sale_price_effective_date",
    "brand",
    "gtin",
    "mpn",
    "identifier_exists",
    "condition",
    "google_product_category",
    "product_type",
    "item_group_id",
    "color",
    "size",
    "size_type",
    "size_system",
    "gender",
    "age_group",
    "material",
    "pattern",
    "shipping",
    "shipping_weight",
    "tax",
    "custom_label_0",
    "custom_label_1",
    "custom_label_2",
    "custom_label_3",
    "custom_label_4",
];

/// Normalize an attribute name to its canonical form: trim, lowercase, strip a
/// leading `g:` namespace prefix, convert spaces/hyphens to underscores, and map
/// a small set of well-known human-facing aliases (e.g. a CSV column literally
/// titled `"Image Link"` or `"Google Product Category"`).
///
/// Aliasing is intentionally conservative — only mappings that are unambiguous
/// in the Merchant Center vocabulary — because an over-eager rename could merge
/// two genuinely distinct columns and lose data.
pub fn canonical_attr(raw: &str) -> String {
    let mut s = raw.trim().to_ascii_lowercase();
    if let Some(rest) = s.strip_prefix("g:") {
        s = rest.to_string();
    }
    let s: String = s
        .chars()
        .map(|c| if c == ' ' || c == '-' { '_' } else { c })
        .collect();
    // Collapse runs of underscores.
    let mut collapsed = String::with_capacity(s.len());
    let mut prev_us = false;
    for c in s.chars() {
        if c == '_' {
            if !prev_us {
                collapsed.push(c);
            }
            prev_us = true;
        } else {
            collapsed.push(c);
            prev_us = false;
        }
    }
    let s = collapsed.trim_matches('_').to_string();

    // Only unambiguous renames live here. Identifier synonyms like UPC/EAN/ISBN
    // are *not* folded into `gtin` — a feed may carry both columns, and merging
    // them would silently drop data. The GTIN rule treats them as equivalents at
    // validation time instead. Likewise "manufacturer" is left alone because it
    // usually means brand, not MPN.
    match s.as_str() {
        "image" | "imagelink" | "image_url" | "imageurl" => "image_link".into(),
        "additional_image" | "additional_image_url" => "additional_image_link".into(),
        "url" | "product_url" | "producturl" | "product_link" => "link".into(),
        "google_category" | "googleproductcategory" => "google_product_category".into(),
        "stock" | "stock_status" | "stock_availability" => "availability".into(),
        "group_id" | "itemgroupid" | "variant_group_id" => "item_group_id".into(),
        "manufacturer_part_number" => "mpn".into(),
        _ => s,
    }
}
