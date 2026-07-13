use greenlit_engine::{audit_auto, AuditOptions, Destination};

fn product(extra: &str) -> String {
    format!(
        r#"<rss xmlns:g="http://base.google.com/ns/1.0"><channel><title>Store</title>
        <item><g:id>P1</g:id><title>A sufficiently descriptive product title</title>
        <description>A sufficiently long and accurate product description for testing.</description>
        <link>https://shop.example/products/p1</link>
        <g:image_link>https://shop.example/images/p1.jpg</g:image_link>
        <g:availability>in_stock</g:availability><g:price>10.00 USD</g:price>
        <g:brand>Example Brand</g:brand><g:mpn>MPN-1</g:mpn>{extra}</item>
        </channel></rss>"#
    )
}

fn audit(xml: &str, country: &str, destination: Destination) -> greenlit_engine::Report {
    audit_auto(
        xml.as_bytes(),
        &AuditOptions {
            target_country: country.into(),
            destination,
            ..AuditOptions::default()
        },
    )
    .unwrap()
}

#[test]
fn structured_description_satisfies_required_description() {
    let xml = product("").replace(
        "<description>A sufficiently long and accurate product description for testing.</description>",
        "<g:structured_description>default:Accurate product description.</g:structured_description>",
    );
    let report = audit(&xml, "US", Destination::ShoppingAds);
    assert!(!report
        .findings
        .iter()
        .any(|finding| finding.rule_id == "GL-REQ-description"));
}

#[test]
fn structured_title_satisfies_required_title() {
    let xml = product("").replace(
        "<title>A sufficiently descriptive product title</title>",
        "<g:structured_title>default:A sufficiently descriptive product title</g:structured_title>",
    );
    let report = audit(&xml, "US", Destination::ShoppingAds);
    assert!(!report
        .findings
        .iter()
        .any(|finding| finding.rule_id == "GL-REQ-title"));
}

#[test]
fn missing_title_and_structured_title_is_disapproved() {
    let xml = product("").replace(
        "<title>A sufficiently descriptive product title</title>",
        "",
    );
    let report = audit(&xml, "US", Destination::ShoppingAds);
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.rule_id == "GL-REQ-title"));
}

#[test]
fn structured_attribute_wrappers_do_not_count_toward_content_limits() {
    let structured_title = format!(
        "<g:structured_title>default:\"{}\"</g:structured_title>",
        "T".repeat(150)
    );
    let structured_description = format!(
        "<g:structured_description>default:\"{}\"</g:structured_description>",
        "D".repeat(5000)
    );
    let xml = product("")
        .replace(
            "<title>A sufficiently descriptive product title</title>",
            &structured_title,
        )
        .replace(
            "<description>A sufficiently long and accurate product description for testing.</description>",
            &structured_description,
        );
    let report = audit(&xml, "US", Destination::ShoppingAds);
    assert!(!report.findings.iter().any(|finding| {
        finding.rule_id == "GL-LEN-structured_title"
            || finding.rule_id == "GL-LEN-structured_description"
    }));
}

#[test]
fn current_size_type_values_are_accepted() {
    for value in ["regular", "petite", "maternity", "big", "tall", "plus"] {
        let xml = product(&format!("<g:size_type>{value}</g:size_type>"));
        let report = audit(&xml, "US", Destination::ShoppingAds);
        assert!(!report
            .findings
            .iter()
            .any(|finding| finding.rule_id == "GL-ENUM-size_type"));
    }
    let report = audit(
        &product("<g:size_type>big_and_tall</g:size_type>"),
        "US",
        Destination::ShoppingAds,
    );
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.rule_id == "GL-ENUM-size_type"));
}

#[test]
fn jewelry_does_not_incorrectly_require_size() {
    let xml = product(
        "<g:google_product_category>Apparel &amp; Accessories &gt; Jewelry</g:google_product_category>\
         <g:color>Gold</g:color><g:gender>unisex</g:gender><g:age_group>adult</g:age_group>",
    );
    let report = audit(&xml, "US", Destination::ShoppingAds);
    assert!(!report
        .findings
        .iter()
        .any(|finding| finding.rule_id == "GL-APPAREL-size"));
}

#[test]
fn apparel_country_and_destination_matrix_is_applied() {
    let xml = product(
        "<g:google_product_category>Apparel &amp; Accessories &gt; Clothing</g:google_product_category>",
    );
    let parsed = greenlit_engine::parse::parse_auto(xml.as_bytes()).unwrap();
    assert_eq!(
        parsed.products[0].get("google_product_category"),
        Some("Apparel & Accessories > Clothing")
    );
    let de_ads = audit(&xml, "DE", Destination::ShoppingAds);
    assert!(
        de_ads
            .findings
            .iter()
            .any(|finding| finding.rule_id == "GL-APPAREL-size"),
        "{:?}",
        de_ads
            .findings
            .iter()
            .map(|finding| &finding.rule_id)
            .collect::<Vec<_>>()
    );

    let ca_ads = audit(&xml, "CA", Destination::ShoppingAds);
    assert!(!ca_ads
        .findings
        .iter()
        .any(|finding| finding.rule_id.starts_with("GL-APPAREL-")));

    let ca_free = audit(&xml, "CA", Destination::FreeListings);
    assert!(ca_free
        .findings
        .iter()
        .any(|finding| finding.rule_id == "GL-APPAREL-size"));
}

#[test]
fn heuristic_apparel_match_never_claims_disapproval() {
    let report = audit(
        &product("<g:product_type>Handmade Shirts</g:product_type>"),
        "US",
        Destination::ShoppingAds,
    );
    let apparel: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.rule_id.starts_with("GL-APPAREL-"))
        .collect();
    assert!(!apparel.is_empty());
    assert!(apparel
        .iter()
        .all(|finding| finding.severity == greenlit_engine::Severity::AtRisk));
}

#[test]
fn invalid_video_link_is_reported_without_disapproving_product() {
    let report = audit(
        &product("<g:video_link>javascript:alert(1)</g:video_link>"),
        "US",
        Destination::ShoppingAds,
    );
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.rule_id == "GL-URL-video_link")
        .unwrap();
    assert_eq!(finding.severity, greenlit_engine::Severity::AtRisk);
}
