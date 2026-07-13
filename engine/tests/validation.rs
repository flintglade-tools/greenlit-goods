use greenlit_engine::error::EngineError;
use greenlit_engine::{audit_auto, AuditOptions};

fn feed(prices: &[(&str, &str)]) -> String {
    let items = prices
        .iter()
        .map(|(id, price)| {
            format!(
                r#"<item><g:id>{id}</g:id><title>A sufficiently descriptive product title</title>
                <description>A sufficiently long and accurate product description for testing.</description>
                <link>https://shop.example/products/{id}</link>
                <g:image_link>https://shop.example/images/{id}.jpg</g:image_link>
                <g:availability>in_stock</g:availability><g:price>{price}</g:price>
                <g:brand>Example Brand</g:brand><g:mpn>MPN-{id}</g:mpn></item>"#
            )
        })
        .collect::<String>();
    format!(
        r#"<rss xmlns:g="http://base.google.com/ns/1.0"><channel><title>Store</title>{items}</channel></rss>"#
    )
}

#[test]
fn non_finite_scientific_and_malformed_prices_are_rejected() {
    for price in ["1e309 USD", "1,2,3.00 USD", "10.001 USD", "USD 10.00"] {
        let report =
            audit_auto(feed(&[("P1", price)]).as_bytes(), &AuditOptions::default()).unwrap();
        assert!(report
            .findings
            .iter()
            .any(|finding| finding.rule_id == "GL-PRICE-FORMAT"));
        assert_eq!(report.red, 1);
    }
}

#[test]
fn invalid_audit_options_fail_before_reporting() {
    let xml = feed(&[("P1", "10.00 USD")]);
    for value in [-1.0, f64::NAN, f64::INFINITY, 1.0e308] {
        let options = AuditOptions {
            assumed_monthly_sales: value,
            ..AuditOptions::default()
        };
        assert!(matches!(
            audit_auto(xml.as_bytes(), &options),
            Err(EngineError::InvalidOptions(_))
        ));
    }
    let options = AuditOptions {
        target_country: "USA".into(),
        ..AuditOptions::default()
    };
    assert!(matches!(
        audit_auto(xml.as_bytes(), &options),
        Err(EngineError::InvalidOptions(_))
    ));

    let options = AuditOptions {
        target_country: "ZZ".into(),
        ..AuditOptions::default()
    };
    assert!(matches!(
        audit_auto(xml.as_bytes(), &options),
        Err(EngineError::InvalidOptions(_))
    ));

    let options = AuditOptions {
        target_country: "us".into(),
        ..AuditOptions::default()
    };
    assert!(audit_auto(xml.as_bytes(), &options).is_ok());
}

#[test]
fn mixed_currency_revenue_is_never_summed() {
    let xml = feed(&[("P1", "10.00 USD"), ("P2", "20.00 EUR")]).replace(
        "<title>A sufficiently descriptive product title</title>",
        "",
    );
    let report = audit_auto(xml.as_bytes(), &AuditOptions::default()).unwrap();
    assert_eq!(report.revenue_at_risk.by_currency.len(), 2);
    assert_eq!(report.revenue_at_risk.by_currency[0].currency, "EUR");
    assert_eq!(report.revenue_at_risk.by_currency[1].currency, "USD");
}

#[test]
fn invalid_currency_is_excluded_from_revenue_estimate() {
    let xml = feed(&[("P1", "10.00 XYZ")]).replace(
        "<title>A sufficiently descriptive product title</title>",
        "",
    );
    let report = audit_auto(xml.as_bytes(), &AuditOptions::default()).unwrap();
    assert!(report.revenue_at_risk.by_currency.is_empty());
    assert_eq!(report.revenue_at_risk.excluded_affected_products, 1);
}
