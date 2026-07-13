//! The unit of diagnosis.
//!
//! Every parser warning and every rule violation is expressed as a [`Finding`].
//! A finding is deliberately self-describing: it names the rule, the offending
//! attribute, the consequence, the spec basis, and whether the engine can fix
//! it automatically. That self-description is what powers both the sales-facing
//! report ("here is each problem and why") and the fixer ("here is what is safe
//! to touch").

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingBasis {
    GoogleSpecification,
    GreenlitHeuristic,
    FeedStructure,
}

impl FindingBasis {
    pub const fn label(self) -> &'static str {
        match self {
            Self::GoogleSpecification => "google_spec",
            Self::GreenlitHeuristic => "heuristic",
            Self::FeedStructure => "structure",
        }
    }
}

/// How badly a finding hurts the merchant, in Google Merchant Center terms.
///
/// The three levels map directly onto the product's red / yellow / green
/// "greenlight" status, which is the metaphor the whole tool is built around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The product will be **disapproved** and will not serve at all
    /// (missing required attribute, invalid GTIN, malformed price, duplicate id).
    /// One of these turns a product red.
    Disapproval,
    /// The product can serve but is **limited or at risk** — a likely warning,
    /// degraded reach, or a condition Google penalizes (no unique identifier,
    /// promotional text in the title, preorder without a date). Turns a product
    /// yellow if it has no disapprovals.
    AtRisk,
    /// The product serves fine but the listing quality could be **optimized**
    /// (ALL-CAPS title, HTML in the description, a very short title). Does not by
    /// itself stop a product being green, but drags the Greenlight Score.
    Optimization,
}

impl Severity {
    pub const fn label(self) -> &'static str {
        match self {
            Severity::Disapproval => "disapproval",
            Severity::AtRisk => "at-risk",
            Severity::Optimization => "optimization",
        }
    }
}

/// A single diagnosed problem on a single product (or, when `product_id` is
/// `None`, a feed-structural problem).
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Stable rule identifier, e.g. `"GL-TITLE-LEN"`. Stable so downstream
    /// tooling, dashboards, and tests can key off it without parsing prose.
    pub rule_id: String,
    pub severity: Severity,
    /// The product `id` this finding belongs to, if known.
    pub product_id: Option<String>,
    /// The offending attribute, e.g. `"title"`. `None` for whole-product issues.
    pub field: Option<String>,
    /// Plain-English statement of what is wrong.
    pub message: String,
    /// The spec basis or extra context (kept separate so the message stays terse).
    pub detail: Option<String>,
    /// True if the engine can correct this safely and automatically. False means
    /// it is surfaced as a *suggestion* for a human — never silently changed.
    pub auto_fixable: bool,
    pub basis: FindingBasis,
    pub source_url: Option<String>,
    pub source_verified_on: Option<String>,
}

impl Finding {
    /// Builder entry point. `product_id` is normally filled in by the engine
    /// after the rule runs, so rules can stay product-agnostic.
    pub fn new(rule_id: impl Into<String>, severity: Severity, message: impl Into<String>) -> Self {
        Finding {
            rule_id: rule_id.into(),
            severity,
            product_id: None,
            field: None,
            message: message.into(),
            detail: None,
            auto_fixable: false,
            basis: FindingBasis::GoogleSpecification,
            source_url: Some(crate::rules::PRODUCT_DATA_SPEC_URL.to_string()),
            source_verified_on: Some(crate::rules::SPEC_VERIFIED_ON.to_string()),
        }
    }

    pub fn field(mut self, f: impl Into<String>) -> Self {
        self.field = Some(f.into());
        self
    }

    pub fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = Some(d.into());
        self
    }

    pub fn fixable(mut self) -> Self {
        self.auto_fixable = true;
        self
    }

    pub fn heuristic(mut self) -> Self {
        self.basis = FindingBasis::GreenlitHeuristic;
        self.source_url = None;
        self.source_verified_on = None;
        self
    }

    pub fn structural(mut self) -> Self {
        self.basis = FindingBasis::FeedStructure;
        self.source_url = None;
        self.source_verified_on = None;
        self
    }

    pub(crate) fn with_product(mut self, id: Option<String>) -> Self {
        if self.product_id.is_none() {
            self.product_id = id;
        }
        self
    }
}
