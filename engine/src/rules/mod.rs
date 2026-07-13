//! The rules engine: a registry of [`Rule`]s run against every product, plus the
//! [`FeedContext`] that carries the cross-product information individual rules
//! need (duplicate-id detection, variant grouping, target market).

pub mod declarative;
pub mod logic;

pub const PRODUCT_DATA_SPEC_URL: &str = "https://support.google.com/merchants/answer/7052112?hl=en";
pub const SPEC_VERIFIED_ON: &str = "2026-07-04";

use std::collections::HashMap;

use crate::finding::Finding;
use crate::model::{Feed, Product};
use crate::report::Destination;

/// Cross-product context computed once per feed, before rules run.
pub struct FeedContext {
    /// ISO 3166-1 alpha-2 target market. Affects conditionally-required
    /// attributes (e.g. apparel requirements are stricter in the US).
    pub target_country: String,
    pub destination: Destination,
    /// How many times each product `id` appears (for duplicate detection).
    pub id_counts: HashMap<String, usize>,
    /// How many products share each `item_group_id` (for variant checks).
    pub group_counts: HashMap<String, usize>,
}

impl FeedContext {
    pub fn build(feed: &Feed, target_country: &str, destination: Destination) -> Self {
        let mut id_counts = HashMap::new();
        let mut group_counts = HashMap::new();
        for p in &feed.products {
            if let Some(id) = p.id() {
                *id_counts.entry(id.to_string()).or_insert(0) += 1;
            }
            if let Some(gid) = p.get("item_group_id") {
                *group_counts.entry(gid.to_string()).or_insert(0) += 1;
            }
        }
        FeedContext {
            target_country: target_country.to_ascii_uppercase(),
            destination,
            id_counts,
            group_counts,
        }
    }
}

/// A single validation rule. Rules are product-agnostic: they receive a product
/// and the feed context, and return findings without knowing which product id
/// they belong to — the runner stamps that on afterward.
pub trait Rule {
    /// Stable category identifier, for debugging and registry introspection.
    fn id(&self) -> &'static str;
    fn check(&self, product: &Product, ctx: &FeedContext) -> Vec<Finding>;
}

/// The full rule set. Order here is the order findings are produced per product.
pub fn all_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(declarative::DeclarativeRule),
        Box::new(logic::GtinRule),
        Box::new(logic::IdentifierRule),
        Box::new(logic::UrlRule),
        Box::new(logic::PriceRule),
        Box::new(logic::PreorderDateRule),
        Box::new(logic::ApparelRule),
        Box::new(logic::VariantRule),
        Box::new(logic::DuplicateIdRule),
        Box::new(logic::TitleQualityRule),
        Box::new(logic::DescriptionQualityRule),
        Box::new(logic::ImagePlaceholderRule),
        Box::new(logic::WhitespaceRule),
    ]
}

/// Run every rule against every product, returning all findings with their
/// owning product id attached.
pub fn run_rules(feed: &Feed, ctx: &FeedContext) -> Vec<Finding> {
    run_rules_grouped(feed, ctx).into_iter().flatten().collect()
}

/// Like [`run_rules`] but keeps each product's findings in their own bucket,
/// aligned with `feed.products` by index. Scoring uses this so it can attribute
/// findings to products that have no `id` (which `id`-keyed matching can't).
pub fn run_rules_grouped(feed: &Feed, ctx: &FeedContext) -> Vec<Vec<Finding>> {
    let rules = all_rules();
    let mut out = Vec::with_capacity(feed.products.len());
    for p in &feed.products {
        let id = p.id().map(str::to_string);
        let mut bucket = Vec::new();
        for rule in &rules {
            for finding in rule.check(p, ctx) {
                bucket.push(finding.with_product(id.clone()));
            }
        }
        out.push(bucket);
    }
    out
}
