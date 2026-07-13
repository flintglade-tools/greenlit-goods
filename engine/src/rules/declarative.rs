//! The declarative half of the rules engine.
//!
//! A flat table of [`FieldSpec`]s expresses the "boring" majority of the Google
//! product spec — which attributes are required, their length caps, and their
//! allowed enumerated values. One [`DeclarativeRule`] walks the table so new
//! requirements are a one-line table edit, not new code.
//!
//! [`normalize_enum`] is deliberately shared with the fixer module: it is the
//! single source of truth for "what does this messy enum value really mean", so
//! the rule's `auto_fixable` flag can never disagree with what the fixer does.

use crate::finding::{Finding, Severity};
use crate::model::Product;
use crate::rules::{FeedContext, Rule};

/// Allowed values are stored lowercased; matching is done after normalization.
struct FieldSpec {
    attr: &'static str,
    required: bool,
    missing_severity: Severity,
    max_len: Option<usize>,
    allowed: Option<&'static [&'static str]>,
}

const SPECS: &[FieldSpec] = &[
    FieldSpec {
        attr: "id",
        required: true,
        missing_severity: Severity::Disapproval,
        max_len: Some(50),
        allowed: None,
    },
    FieldSpec {
        attr: "title",
        required: true,
        missing_severity: Severity::Disapproval,
        max_len: Some(150),
        allowed: None,
    },
    FieldSpec {
        attr: "description",
        required: false,
        missing_severity: Severity::Disapproval,
        max_len: Some(5000),
        allowed: None,
    },
    FieldSpec {
        attr: "structured_description",
        required: false,
        missing_severity: Severity::Disapproval,
        max_len: Some(5000),
        allowed: None,
    },
    FieldSpec {
        attr: "link",
        required: true,
        missing_severity: Severity::Disapproval,
        max_len: Some(2000),
        allowed: None,
    },
    FieldSpec {
        attr: "image_link",
        required: true,
        missing_severity: Severity::Disapproval,
        max_len: Some(2000),
        allowed: None,
    },
    FieldSpec {
        attr: "availability",
        required: true,
        missing_severity: Severity::Disapproval,
        max_len: None,
        allowed: Some(&["in_stock", "out_of_stock", "preorder", "backorder"]),
    },
    FieldSpec {
        attr: "price",
        required: true,
        missing_severity: Severity::Disapproval,
        max_len: None,
        allowed: None,
    },
    // Conditionally meaningful, but when present the value must be valid.
    FieldSpec {
        attr: "condition",
        required: false,
        missing_severity: Severity::Optimization,
        max_len: None,
        allowed: Some(&["new", "refurbished", "used"]),
    },
    FieldSpec {
        attr: "gender",
        required: false,
        missing_severity: Severity::Optimization,
        max_len: None,
        allowed: Some(&["male", "female", "unisex"]),
    },
    FieldSpec {
        attr: "age_group",
        required: false,
        missing_severity: Severity::Optimization,
        max_len: None,
        allowed: Some(&["newborn", "infant", "toddler", "kids", "adult"]),
    },
    FieldSpec {
        attr: "size_type",
        required: false,
        missing_severity: Severity::Optimization,
        max_len: None,
        allowed: Some(&["regular", "petite", "maternity", "big", "tall", "plus"]),
    },
    FieldSpec {
        attr: "video_link",
        required: false,
        missing_severity: Severity::Optimization,
        max_len: Some(2000),
        allowed: None,
    },
    FieldSpec {
        attr: "adult",
        required: false,
        missing_severity: Severity::Optimization,
        max_len: None,
        allowed: Some(&["yes", "no"]),
    },
    FieldSpec {
        attr: "is_bundle",
        required: false,
        missing_severity: Severity::Optimization,
        max_len: None,
        allowed: Some(&["yes", "no"]),
    },
    FieldSpec {
        attr: "identifier_exists",
        required: false,
        missing_severity: Severity::Optimization,
        max_len: None,
        allowed: Some(&["yes", "no"]),
    },
];

pub struct DeclarativeRule;

impl Rule for DeclarativeRule {
    fn id(&self) -> &'static str {
        "declarative"
    }

    fn check(&self, p: &Product, _ctx: &FeedContext) -> Vec<Finding> {
        let mut out = Vec::new();
        for spec in SPECS {
            match p.get(spec.attr) {
                None => {
                    if spec.required {
                        out.push(
                            Finding::new(
                                format!("GL-REQ-{}", spec.attr),
                                spec.missing_severity,
                                format!("Required attribute '{}' is missing or blank.", spec.attr),
                            )
                            .field(spec.attr)
                            .detail("Google disapproves products missing required attributes."),
                        );
                    }
                }
                Some(value) => {
                    if let Some(max) = spec.max_len {
                        let len = value.chars().count();
                        if len > max {
                            out.push(
                                Finding::new(
                                    format!("GL-LEN-{}", spec.attr),
                                    Severity::AtRisk,
                                    format!(
                                        "'{}' is {len} characters; the limit is {max}.",
                                        spec.attr
                                    ),
                                )
                                .field(spec.attr)
                                .detail("Over-limit values are truncated or rejected by Google."),
                            );
                        }
                    }
                    if let Some(allowed) = spec.allowed {
                        match enum_canonical(spec.attr, value) {
                            Some(canon) if canon != value => out.push(
                                Finding::new(
                                    format!("GL-ENUM-{}", spec.attr),
                                    Severity::AtRisk,
                                    format!(
                                        "'{}' value \"{value}\" should be \"{canon}\".",
                                        spec.attr
                                    ),
                                )
                                .field(spec.attr)
                                .detail("Recognized variant; can be normalized automatically.")
                                .fixable(),
                            ),
                            Some(_) => {} // already in canonical form
                            None => out.push(
                                Finding::new(
                                    format!("GL-ENUM-{}", spec.attr),
                                    Severity::Disapproval,
                                    format!(
                                        "'{}' has invalid value \"{value}\"; allowed: {}.",
                                        spec.attr,
                                        allowed.join(", ")
                                    ),
                                )
                                .field(spec.attr)
                                .detail("Unrecognized enumerated value; Google will disapprove."),
                            ),
                        }
                    }
                }
            }
        }
        if p.get("description").is_none() && p.get("structured_description").is_none() {
            out.push(
                Finding::new(
                    "GL-REQ-description",
                    Severity::Disapproval,
                    "One of 'description' or 'structured_description' is required.",
                )
                .field("description")
                .detail("Google accepts either description attribute."),
            );
        }
        out
    }
}

/// Generic normalization for enum comparison: trim, lowercase, unify separators,
/// collapse repeats. (`"In Stock"` and `"in-stock"` both become `"in_stock"`.)
pub(crate) fn generic_normalize(s: &str) -> String {
    let lowered = s.trim().to_lowercase();
    let mapped: String = lowered
        .chars()
        .map(|c| if c == ' ' || c == '-' { '_' } else { c })
        .collect();
    let mut out = String::with_capacity(mapped.len());
    let mut prev_us = false;
    for c in mapped.chars() {
        if c == '_' {
            if !prev_us {
                out.push(c);
            }
            prev_us = true;
        } else {
            out.push(c);
            prev_us = false;
        }
    }
    out.trim_matches('_').to_string()
}

/// Map a messy enum value to its canonical Google value, if it can be done
/// *safely*. Returns `None` when the intent is ambiguous (in which case the rule
/// reports a hard disapproval rather than guessing). Shared by the fixer.
pub(crate) fn normalize_enum(attr: &str, raw: &str) -> Option<&'static str> {
    let n = generic_normalize(raw);
    match attr {
        "availability" => match n.as_str() {
            "in_stock" | "instock" | "available" | "in_stock_online" => Some("in_stock"),
            "out_of_stock" | "outofstock" | "sold_out" | "soldout" | "unavailable" => {
                Some("out_of_stock")
            }
            "preorder" | "pre_order" => Some("preorder"),
            "backorder" | "back_order" | "backordered" => Some("backorder"),
            _ => None,
        },
        "condition" => match n.as_str() {
            "new" | "brand_new" | "brandnew" => Some("new"),
            "refurbished" | "refurb" | "renewed" | "reconditioned" => Some("refurbished"),
            "used" | "pre_owned" | "preowned" | "second_hand" | "secondhand" => Some("used"),
            _ => None,
        },
        "gender" => match n.as_str() {
            "male" | "men" | "man" | "mens" | "boys" | "boy" => Some("male"),
            "female" | "women" | "woman" | "womens" | "girls" | "girl" => Some("female"),
            "unisex" | "uni" | "both" => Some("unisex"),
            _ => None,
        },
        "age_group" => match n.as_str() {
            "newborn" => Some("newborn"),
            "infant" | "baby" => Some("infant"),
            "toddler" => Some("toddler"),
            "kids" | "kid" | "child" | "children" | "junior" => Some("kids"),
            "adult" | "adults" => Some("adult"),
            _ => None,
        },
        "adult" | "is_bundle" | "identifier_exists" => match n.as_str() {
            "yes" | "true" | "1" | "y" => Some("yes"),
            "no" | "false" | "0" | "n" => Some("no"),
            _ => None,
        },
        _ => None,
    }
}

/// The single source of truth for "what canonical Google value does this enum
/// string mean", shared by the rule and the fixer so they never disagree.
///
/// Resolution order: exact match against the allowed set; then a synonym map
/// ([`normalize_enum`]); then a case/separator-only normalization. Returns the
/// canonical allowed value (equal to the trimmed input when already canonical),
/// or `None` when the value cannot be safely interpreted.
pub(crate) fn enum_canonical(attr: &str, raw: &str) -> Option<&'static str> {
    let allowed = allowed_for(attr)?;
    let trimmed = raw.trim();
    if let Some(exact) = allowed.iter().find(|a| **a == trimmed) {
        return Some(*exact);
    }
    if let Some(c) = normalize_enum(attr, raw) {
        return Some(c);
    }
    let gen = generic_normalize(raw);
    allowed.iter().find(|a| **a == gen).copied()
}

fn allowed_for(attr: &str) -> Option<&'static [&'static str]> {
    SPECS
        .iter()
        .find(|s| s.attr == attr)
        .and_then(|s| s.allowed)
}

/// The set of attributes the fixer should attempt enum-normalization on — every
/// attribute in the spec table that defines an allowed value set.
pub(crate) const ENUM_ATTRS: &[&str] = &[
    "availability",
    "condition",
    "gender",
    "age_group",
    "size_type",
    "adult",
    "is_bundle",
    "identifier_exists",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_canonical_resolves_variants() {
        // already canonical
        assert_eq!(enum_canonical("availability", "in_stock"), Some("in_stock"));
        // case/separator variant
        assert_eq!(enum_canonical("availability", "In Stock"), Some("in_stock"));
        assert_eq!(enum_canonical("availability", "IN-STOCK"), Some("in_stock"));
        // synonym
        assert_eq!(
            enum_canonical("availability", "available"),
            Some("in_stock")
        );
        assert_eq!(enum_canonical("condition", "Brand New"), Some("new"));
        assert_eq!(enum_canonical("condition", "pre-owned"), Some("used"));
        assert_eq!(enum_canonical("gender", "Womens"), Some("female"));
        assert_eq!(enum_canonical("identifier_exists", "TRUE"), Some("yes"));
        // unresolvable -> None (rule will treat as disapproval)
        assert_eq!(enum_canonical("availability", "banana"), None);
        assert_eq!(enum_canonical("condition", "mint"), None);
        // unknown attribute -> None
        assert_eq!(enum_canonical("not_an_enum", "x"), None);
    }

    #[test]
    fn generic_normalize_unifies_separators_and_case() {
        assert_eq!(generic_normalize("In Stock"), "in_stock");
        assert_eq!(generic_normalize("  PRE-ORDER "), "pre_order");
        assert_eq!(generic_normalize("a__b___c"), "a_b_c");
    }
}
