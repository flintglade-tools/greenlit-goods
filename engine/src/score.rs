//! Scoring and the revenue-at-risk estimate.
//!
//! The Greenlight Score rolls the findings up into one 0–100 number a merchant
//! can watch over time, and the per-product red/yellow/green status is the
//! metaphor the whole tool turns on. Revenue-at-risk is deliberately framed as
//! an *estimate driven by an explicit, tunable assumption* — never a promise —
//! because we cannot know a merchant's real traffic from the feed alone.

use serde::Serialize;

use crate::finding::{Finding, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// No disapprovals and no at-risk findings.
    Green,
    /// At-risk findings but no disapprovals — serves, but limited.
    Yellow,
    /// One or more disapprovals — will not serve.
    Red,
}

/// Per-severity tally for a single product.
#[derive(Debug, Clone, Copy, Default)]
pub struct Counts {
    pub disapprovals: usize,
    pub at_risk: usize,
    pub optimizations: usize,
}

impl Counts {
    pub fn from_findings(findings: &[Finding]) -> Self {
        let mut c = Counts::default();
        for f in findings {
            match f.severity {
                Severity::Disapproval => c.disapprovals += 1,
                Severity::AtRisk => c.at_risk += 1,
                Severity::Optimization => c.optimizations += 1,
            }
        }
        c
    }

    pub fn status(&self) -> Status {
        if self.disapprovals > 0 {
            Status::Red
        } else if self.at_risk > 0 {
            Status::Yellow
        } else {
            Status::Green
        }
    }

    /// A 0–100 health score for one product. A disapproval is heavily weighted
    /// (it stops the product serving), an at-risk finding moderately, an
    /// optimization lightly. Clamped at zero.
    pub fn score(&self) -> u32 {
        let penalty = 40 * self.disapprovals + 10 * self.at_risk + 2 * self.optimizations;
        100u32.saturating_sub(penalty.min(100) as u32)
    }
}

/// The feed-level Greenlight Score: the mean of per-product scores, or 100 for an
/// empty product set. Rounded to the nearest integer.
pub fn greenlight_score(per_product: &[Counts]) -> u32 {
    if per_product.is_empty() {
        return 100;
    }
    let total: u32 = per_product.iter().map(|c| c.score()).sum();
    ((total as f64) / (per_product.len() as f64)).round() as u32
}

/// The result of the revenue-at-risk estimate, carrying its own assumptions so
/// the caller can show exactly how the number was produced.
#[derive(Debug, Clone, Serialize)]
pub struct CurrencyRevenue {
    pub currency: String,
    pub disapproved_monthly: f64,
    pub at_risk_monthly: f64,
    pub total_monthly: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RevenueAtRisk {
    pub by_currency: Vec<CurrencyRevenue>,
    pub assumed_monthly_sales_per_product: f64,
    /// Fraction of full exposure attributed to a yellow (at-risk) product.
    pub at_risk_weight: f64,
    pub excluded_affected_products: usize,
    pub assumption: String,
}

/// Estimate monthly revenue exposure from disapproved and at-risk products.
///
/// `prices` is the parsed price amount for each product (in the product's own
/// currency), aligned with `statuses`. The model is intentionally simple and
/// stated outright: a red product risks its full `price × assumed_sales`; a
/// yellow product risks `at_risk_weight` of that. The point is a defensible
/// order-of-magnitude figure the merchant can recompute with their real numbers,
/// not a fake-precise forecast.
pub fn estimate_revenue(
    prices: &[Option<i64>],
    statuses: &[Status],
    currencies: &[Option<String>],
    assumed_monthly_sales_per_product: f64,
) -> RevenueAtRisk {
    const AT_RISK_WEIGHT: f64 = 0.3;

    let assumed_sales = assumed_monthly_sales_per_product.max(0.0);
    let mut totals: std::collections::BTreeMap<String, (f64, f64)> =
        std::collections::BTreeMap::new();
    let mut excluded = 0usize;
    for ((price, status), currency) in prices.iter().zip(statuses).zip(currencies) {
        if *status == Status::Green {
            continue;
        }
        let (Some(cents), Some(currency)) = (price, currency.as_deref()) else {
            excluded += 1;
            continue;
        };
        if *cents <= 0 {
            excluded += 1;
            continue;
        }
        let p = (*cents as f64) / 100.0;
        let entry = totals.entry(currency.to_string()).or_default();
        match status {
            Status::Red => entry.0 += p * assumed_sales,
            Status::Yellow => entry.1 += p * assumed_sales * AT_RISK_WEIGHT,
            Status::Green => {}
        }
    }
    let by_currency = totals
        .into_iter()
        .map(|(currency, (disapproved, at_risk))| CurrencyRevenue {
            currency,
            disapproved_monthly: round2(disapproved),
            at_risk_monthly: round2(at_risk),
            total_monthly: round2(disapproved + at_risk),
        })
        .collect();

    RevenueAtRisk {
        by_currency,
        assumed_monthly_sales_per_product,
        at_risk_weight: AT_RISK_WEIGHT,
        excluded_affected_products: excluded,
        assumption: format!(
            "Assumes each affected product would sell ~{assumed_monthly_sales_per_product} \
             unit(s)/month; red products counted at full price, yellow at {}%. Adjust with your \
             real conversion data.",
            (AT_RISK_WEIGHT * 100.0) as u32
        ),
    }
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(d: usize, r: usize, o: usize) -> Counts {
        Counts {
            disapprovals: d,
            at_risk: r,
            optimizations: o,
        }
    }

    #[test]
    fn per_product_score_follows_weights() {
        assert_eq!(counts(0, 0, 0).score(), 100);
        assert_eq!(counts(1, 0, 0).score(), 60); // one disapproval = -40
        assert_eq!(counts(0, 1, 0).score(), 90); // one at-risk = -10
        assert_eq!(counts(0, 0, 1).score(), 98); // one optimization = -2
        assert_eq!(counts(1, 2, 1).score(), 38); // 40 + 20 + 2 = 62
                                                 // Penalty is clamped so the score floors at zero, never negative.
        assert_eq!(counts(3, 0, 0).score(), 0);
        assert_eq!(counts(99, 0, 0).score(), 0);
    }

    #[test]
    fn status_precedence_is_disapproval_then_at_risk() {
        assert_eq!(counts(0, 0, 0).status(), Status::Green);
        assert_eq!(counts(0, 0, 5).status(), Status::Green); // optimizations alone stay green
        assert_eq!(counts(0, 1, 0).status(), Status::Yellow);
        assert_eq!(counts(1, 9, 9).status(), Status::Red); // any disapproval => red
    }

    #[test]
    fn greenlight_score_is_the_mean() {
        assert_eq!(greenlight_score(&[]), 100); // empty product set
        assert_eq!(greenlight_score(&[counts(0, 0, 0), counts(0, 0, 0)]), 100);
        assert_eq!(greenlight_score(&[counts(0, 0, 0), counts(1, 0, 0)]), 80); // mean(100, 60)
        assert_eq!(
            greenlight_score(&[counts(1, 0, 0), counts(1, 0, 0), counts(0, 0, 0)]),
            73
        ); // mean(60,60,100)=73.3 -> 73
    }

    #[test]
    fn revenue_model_weights_red_full_and_yellow_partial() {
        let prices = [Some(10_000), Some(5_000), Some(2_000)];
        let statuses = [Status::Red, Status::Yellow, Status::Green];
        let cur = [
            Some("USD".to_string()),
            Some("USD".to_string()),
            Some("USD".to_string()),
        ];
        let r = estimate_revenue(&prices, &statuses, &cur, 1.0);
        let usd = &r.by_currency[0];
        assert_eq!(usd.disapproved_monthly, 100.0); // red at full price
        assert_eq!(usd.at_risk_monthly, 15.0); // yellow at 30% of 50
        assert_eq!(usd.total_monthly, 115.0);
        assert_eq!(usd.currency, "USD");
        assert_eq!(r.at_risk_weight, 0.3);
    }

    #[test]
    fn revenue_scales_with_assumed_sales_and_skips_green() {
        let prices = [Some(500), Some(100_000)];
        let statuses = [Status::Red, Status::Green]; // green contributes nothing
        let cur = [Some("USD".to_string()), Some("USD".to_string())];
        let r = estimate_revenue(&prices, &statuses, &cur, 10.0);
        assert_eq!(r.by_currency[0].disapproved_monthly, 50.0); // 5 * 10 units
        assert_eq!(r.by_currency[0].total_monthly, 50.0);
    }

    #[test]
    fn revenue_flags_mixed_currencies_and_picks_dominant() {
        let prices = [Some(1_000), Some(1_000), Some(1_000)];
        let statuses = [Status::Red, Status::Red, Status::Red];
        let cur = [
            Some("USD".to_string()),
            Some("USD".to_string()),
            Some("EUR".to_string()),
        ];
        let r = estimate_revenue(&prices, &statuses, &cur, 1.0);
        assert_eq!(r.by_currency.len(), 2);
        assert_eq!(r.by_currency[0].currency, "EUR");
        assert_eq!(r.by_currency[0].total_monthly, 10.0);
        assert_eq!(r.by_currency[1].currency, "USD");
        assert_eq!(r.by_currency[1].total_monthly, 20.0);
    }

    #[test]
    fn negative_or_missing_prices_do_not_subtract() {
        let prices = [Some(-500), None];
        let statuses = [Status::Red, Status::Red];
        let cur = [None, None];
        let r = estimate_revenue(&prices, &statuses, &cur, 1.0);
        assert!(r.by_currency.is_empty());
        assert_eq!(r.excluded_affected_products, 2);
    }
}
