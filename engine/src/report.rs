//! Assembling the analysis into a [`Report`] and rendering it for a terminal.
//!
//! The report is the product's selling artifact: it states the Greenlight Score,
//! the red/yellow/green breakdown, the estimated revenue exposure, and every
//! finding with its fix. The terminal renderer here is plain text and pure (it
//! returns a `String`); the CLI layers ANSI color on top when attached to a tty.

use serde::Serialize;

use crate::finding::{Finding, Severity};
use crate::model::{Feed, Format};
use crate::rules::logic::price_parts;
use crate::rules::{run_rules_grouped, FeedContext};
use crate::score::{estimate_revenue, greenlight_score, Counts, RevenueAtRisk, Status};

pub const REPORT_SCHEMA_VERSION: u32 = 1;
pub const SCORE_MODEL_VERSION: &str = "1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Destination {
    ShoppingAds,
    FreeListings,
}

/// Options controlling an audit.
#[derive(Debug, Clone)]
pub struct AuditOptions {
    /// ISO 3166-1 alpha-2 target market (affects conditional requirements).
    pub target_country: String,
    /// The tunable assumption behind revenue-at-risk: units/month per affected
    /// product. Stated openly in the output.
    pub assumed_monthly_sales: f64,
    /// Google surface whose conditional requirements should be applied.
    pub destination: Destination,
}

impl Default for AuditOptions {
    fn default() -> Self {
        AuditOptions {
            target_country: "US".to_string(),
            assumed_monthly_sales: 1.0,
            destination: Destination::ShoppingAds,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProductReport {
    pub id: Option<String>,
    pub status: Status,
    pub score: u32,
    pub disapprovals: usize,
    pub at_risk: usize,
    pub optimizations: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuleSummaryItem {
    pub rule_id: String,
    pub severity: Severity,
    pub count: usize,
    pub auto_fixable: bool,
    pub example: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub score_model_version: &'static str,
    pub specification_url: &'static str,
    pub specification_verified_on: &'static str,
    pub format: Format,
    pub target_country: String,
    pub destination: Destination,
    pub product_count: usize,
    /// `None` when the parser recovered only a partial feed. A partial product
    /// set cannot support an honest feed-level score.
    pub greenlight_score: Option<u32>,
    pub structurally_complete: bool,
    pub rewrite_safe: bool,
    pub rewrite_blockers: Vec<String>,
    pub green: usize,
    pub yellow: usize,
    pub red: usize,
    pub total_disapprovals: usize,
    pub total_at_risk: usize,
    pub total_optimizations: usize,
    pub auto_fixable_findings: usize,
    pub revenue_at_risk: RevenueAtRisk,
    pub rule_summary: Vec<RuleSummaryItem>,
    pub products: Vec<ProductReport>,
    pub findings: Vec<Finding>,
    pub notes: Vec<String>,
}

/// Run the rules and build the full report.
pub fn build_report(feed: &Feed, ctx: &FeedContext, opts: &AuditOptions) -> Report {
    let grouped = run_rules_grouped(feed, ctx);

    let mut counts_vec: Vec<Counts> = Vec::with_capacity(feed.products.len());
    let mut statuses: Vec<Status> = Vec::with_capacity(feed.products.len());
    let mut prices: Vec<Option<i64>> = Vec::with_capacity(feed.products.len());
    let mut currencies: Vec<Option<String>> = Vec::with_capacity(feed.products.len());
    let mut products: Vec<ProductReport> = Vec::with_capacity(feed.products.len());

    // Findings list begins with feed-structural parse findings.
    let mut findings: Vec<Finding> = feed.parse_findings.clone();

    for (i, p) in feed.products.iter().enumerate() {
        let f = &grouped[i];
        let counts = Counts::from_findings(f);
        let status = counts.status();
        let (amt, cur) = p.get("price").map(price_parts).unwrap_or((None, None));

        products.push(ProductReport {
            id: p.id().map(str::to_string),
            status,
            score: counts.score(),
            disapprovals: counts.disapprovals,
            at_risk: counts.at_risk,
            optimizations: counts.optimizations,
        });

        counts_vec.push(counts);
        statuses.push(status);
        prices.push(amt);
        currencies.push(cur);
        findings.extend(f.iter().cloned());
    }

    // Severity totals across products plus feed-level parse findings.
    let parse_counts = Counts::from_findings(&feed.parse_findings);
    let total_disapprovals =
        counts_vec.iter().map(|c| c.disapprovals).sum::<usize>() + parse_counts.disapprovals;
    let total_at_risk = counts_vec.iter().map(|c| c.at_risk).sum::<usize>() + parse_counts.at_risk;
    let total_optimizations =
        counts_vec.iter().map(|c| c.optimizations).sum::<usize>() + parse_counts.optimizations;
    let auto_fixable_findings = findings.iter().filter(|f| f.auto_fixable).count();

    let green = statuses.iter().filter(|s| **s == Status::Green).count();
    let yellow = statuses.iter().filter(|s| **s == Status::Yellow).count();
    let red = statuses.iter().filter(|s| **s == Status::Red).count();

    let revenue_at_risk =
        estimate_revenue(&prices, &statuses, &currencies, opts.assumed_monthly_sales);

    let rule_summary = summarize_rules(&findings);
    let notes = build_notes(feed, &revenue_at_risk);

    Report {
        schema_version: REPORT_SCHEMA_VERSION,
        score_model_version: SCORE_MODEL_VERSION,
        specification_url: crate::rules::PRODUCT_DATA_SPEC_URL,
        specification_verified_on: crate::rules::SPEC_VERIFIED_ON,
        format: feed.format,
        target_country: opts.target_country.to_ascii_uppercase(),
        destination: opts.destination,
        product_count: feed.products.len(),
        greenlight_score: feed
            .structurally_complete
            .then(|| greenlight_score(&counts_vec)),
        structurally_complete: feed.structurally_complete,
        rewrite_safe: feed.rewrite_safe(),
        rewrite_blockers: feed.rewrite_blockers.clone(),
        green,
        yellow,
        red,
        total_disapprovals,
        total_at_risk,
        total_optimizations,
        auto_fixable_findings,
        revenue_at_risk,
        rule_summary,
        products,
        findings,
        notes,
    }
}

fn summarize_rules(findings: &[Finding]) -> Vec<RuleSummaryItem> {
    use std::collections::HashMap;
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, RuleSummaryItem> = HashMap::new();
    for f in findings {
        match map.get_mut(&f.rule_id) {
            Some(item) => {
                item.count += 1;
                item.auto_fixable |= f.auto_fixable;
            }
            None => {
                order.push(f.rule_id.clone());
                map.insert(
                    f.rule_id.clone(),
                    RuleSummaryItem {
                        rule_id: f.rule_id.clone(),
                        severity: f.severity,
                        count: 1,
                        auto_fixable: f.auto_fixable,
                        example: f.message.clone(),
                    },
                );
            }
        }
    }
    let mut items: Vec<RuleSummaryItem> =
        order.into_iter().filter_map(|k| map.remove(&k)).collect();
    // Disapprovals first (Severity Ord), then by frequency.
    items.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(b.count.cmp(&a.count))
            .then(a.rule_id.cmp(&b.rule_id))
    });
    items
}

fn build_notes(feed: &Feed, rev: &RevenueAtRisk) -> Vec<String> {
    let mut notes = vec![
        "Greenlit Goods checks a documented, feed-local subset of the published Google \
         Merchant Center product data specification. A result describes this version's \
         supported checks; it does not predict Google's approval decision."
            .to_string(),
        "Apparel detection and placeholder-image checks are heuristic and may over- or \
         under-match; review those findings before bulk action."
            .to_string(),
        "Each JSON finding identifies whether it comes from the Google specification, a Greenlit heuristic, or feed structure, with dated source metadata for normative rules."
            .to_string(),
        rev.assumption.clone(),
    ];
    if rev.by_currency.len() > 1 {
        notes.push(
            "The feed mixes currencies; revenue-at-risk is reported separately per currency and is never summed across them."
                .to_string(),
        );
    }
    if rev.excluded_affected_products > 0 {
        notes.push(format!(
            "{} affected product(s) were excluded from revenue-at-risk because they lacked a valid positive price and ISO currency.",
            rev.excluded_affected_products
        ));
    }
    if !feed.parse_findings.is_empty() {
        notes.push(format!(
            "{} feed-level structural issue(s) were found during parsing and are included in \
             the findings list.",
            feed.parse_findings.len()
        ));
    }
    if !feed.structurally_complete {
        notes.push(
            "The feed is structurally incomplete. No Greenlight Score is reported, strict mode fails, and rewriting is disabled."
                .to_string(),
        );
    } else if !feed.rewrite_safe() {
        notes.push(format!(
            "Audit-only input: rewriting is disabled because {}.",
            feed.rewrite_blockers.join("; ")
        ));
    }
    notes
}

// ---------------------------------------------------------------------------
// Terminal rendering (plain text; CLI adds color)
// ---------------------------------------------------------------------------

/// Render a human-readable report. Pure and deterministic. When `color` is true
/// it embeds ANSI color for the score and status (the CLI passes true on a tty).
pub fn render_terminal(r: &Report, color: bool) -> String {
    let mut s = String::new();
    s.push('\n');
    s.push_str("  GREENLIT GOODS — feed audit\n");
    s.push_str("  ==========================\n\n");

    if let Some(score) = r.greenlight_score {
        let score_col = tier_color(score);
        s.push_str(&format!(
            "  Greenlight Score: {}  {}\n",
            paint(&format!("{score}/100"), score_col, color),
            paint(&score_bar(score), score_col, color),
        ));
    } else {
        s.push_str("  Greenlight Score: N/A  [feed structurally incomplete]\n");
    }
    s.push_str(&format!(
        "  Products: {}   format: {:?}   market: {}   destination: {:?}\n\n",
        r.product_count, r.format, r.target_country, r.destination
    ));

    s.push_str(&format!(
        "  Status:  {}   {}   {}\n",
        paint(&format!("{} green", r.green), "32", color),
        paint(&format!("{} yellow", r.yellow), "33", color),
        paint(&format!("{} red", r.red), "31", color),
    ));
    s.push_str(&format!(
        "  Issues:  {} disapprovals   {} at-risk   {} optimizations   ({} auto-fixable)\n\n",
        r.total_disapprovals, r.total_at_risk, r.total_optimizations, r.auto_fixable_findings
    ));

    let rev = &r.revenue_at_risk;
    if rev.by_currency.is_empty() {
        s.push_str("  Est. revenue at risk: unavailable (no valid affected prices)\n\n");
    } else {
        s.push_str("  Est. revenue at risk (kept separate by currency):\n");
        for currency in &rev.by_currency {
            s.push_str(&format!(
                "    ~{:.2} {} / month ({:.2} disapproved, {:.2} at-risk)\n",
                currency.total_monthly,
                currency.currency,
                currency.disapproved_monthly,
                currency.at_risk_monthly
            ));
        }
        s.push('\n');
    }

    if r.rule_summary.is_empty() && r.structurally_complete {
        s.push_str("  No issues found in this version's supported checks.\n\n");
    } else {
        s.push_str("  Top issues\n");
        s.push_str("  ----------\n");
        s.push_str(&format!(
            "  {:<28} {:<12} {:>6}  {:<4}\n",
            "rule", "severity", "count", "fix?"
        ));
        for item in r.rule_summary.iter().take(15) {
            s.push_str(&format!(
                "  {:<28} {:<12} {:>6}  {:<4}\n",
                truncate(&item.rule_id, 28),
                item.severity.label(),
                item.count,
                if item.auto_fixable { "auto" } else { "—" }
            ));
        }
        s.push('\n');

        // A few worst (red) products to make the impact concrete.
        let mut worst: Vec<&ProductReport> = r
            .products
            .iter()
            .filter(|p| p.status == Status::Red)
            .collect();
        worst.sort_by_key(|p| p.score);
        if !worst.is_empty() {
            s.push_str("  Worst products\n");
            s.push_str("  --------------\n");
            for p in worst.iter().take(8) {
                s.push_str(&format!(
                    "  [{:>3}/100] {:<24} {} disapproval(s), {} at-risk\n",
                    p.score,
                    truncate(p.id.as_deref().unwrap_or("(no id)"), 24),
                    p.disapprovals,
                    p.at_risk
                ));
            }
            s.push('\n');
        }
        s.push_str(&format!(
            "  Summary view shown. Use --json for all {} finding(s), including basis and source metadata.\n\n",
            r.findings.len()
        ));
    }

    s.push_str("  Notes\n");
    s.push_str("  -----\n");
    for note in &r.notes {
        s.push_str(&format!("  • {}\n", wrap_indent(note, 76, "    ")));
    }
    s.push('\n');
    s
}

fn paint(s: &str, code: &str, color: bool) -> String {
    if color {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

/// ANSI color for a score tier: green ≥80, yellow ≥50, red below.
fn tier_color(score: u32) -> &'static str {
    if score >= 80 {
        "32"
    } else if score >= 50 {
        "33"
    } else {
        "31"
    }
}

fn score_bar(score: u32) -> String {
    let filled = (score as usize * 20 / 100).min(20);
    let bar: String = "█".repeat(filled) + &"░".repeat(20 - filled);
    format!("[{bar}]")
}

fn truncate(s: &str, max: usize) -> String {
    let safe = terminal_safe(s);
    if safe.chars().count() <= max {
        safe
    } else {
        let t: String = safe.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

fn terminal_safe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\n' | '\r' | '\t' => out.push(' '),
            ch if ch.is_control() => out.push_str(&format!("\\u{{{:x}}}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out
}

/// Wrap a long note to width, indenting continuation lines.
fn wrap_indent(s: &str, width: usize, indent: &str) -> String {
    let mut out = String::new();
    let mut line_len = 0usize;
    for (i, word) in s.split_whitespace().enumerate() {
        let wl = word.chars().count();
        if i == 0 {
            out.push_str(word);
            line_len = wl;
        } else if line_len + 1 + wl > width {
            out.push('\n');
            out.push_str(indent);
            out.push_str(word);
            line_len = wl;
        } else {
            out.push(' ');
            out.push_str(word);
            line_len += 1 + wl;
        }
    }
    out
}
