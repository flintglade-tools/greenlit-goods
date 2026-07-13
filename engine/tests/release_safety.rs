use greenlit_engine::error::EngineError;
use greenlit_engine::model::Format;
use greenlit_engine::parse::detect_format;
use greenlit_engine::{audit_auto, fix_auto, AuditOptions, FixOptions};

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

#[test]
fn nested_xml_is_auditable_but_never_rewritten() {
    let xml =
        product("<g:shipping><g:country>US</g:country><g:price>4.99 USD</g:price></g:shipping>");
    let report = audit_auto(xml.as_bytes(), &AuditOptions::default()).unwrap();
    assert!(report.structurally_complete);
    assert!(!report.rewrite_safe);
    assert!(matches!(
        fix_auto(xml.as_bytes(), &FixOptions::default()),
        Err(EngineError::UnsafeRewrite(_))
    ));
}

#[test]
fn truncated_xml_has_no_score_and_cannot_be_rewritten() {
    let xml = product("").replace(
        "</channel></rss>",
        "<item><g:id>P2</g:id><title>unfinished</g:price",
    );
    let report = audit_auto(xml.as_bytes(), &AuditOptions::default()).unwrap();
    assert!(!report.structurally_complete);
    assert_eq!(report.greenlight_score, None);
    assert!(matches!(
        fix_auto(xml.as_bytes(), &FixOptions::default()),
        Err(EngineError::UnsafeRewrite(_))
    ));
}

#[test]
fn custom_namespaces_are_audit_only() {
    let xml = product("<c:margin xmlns:c=\"http://base.google.com/cns/1.0\">high</c:margin>");
    let report = audit_auto(xml.as_bytes(), &AuditOptions::default()).unwrap();
    assert!(report.structurally_complete);
    assert!(!report.rewrite_safe);
    assert!(report
        .rewrite_blockers
        .iter()
        .any(|reason| reason.contains("custom namespace")));
}

#[test]
fn rss_without_the_google_namespace_is_not_scored_or_rewritten() {
    let xml = product("").replace(" xmlns:g=\"http://base.google.com/ns/1.0\"", "");
    let report = audit_auto(xml.as_bytes(), &AuditOptions::default()).unwrap();
    assert!(!report.structurally_complete);
    assert_eq!(report.greenlight_score, None);
    assert!(report
        .findings
        .iter()
        .any(|finding| finding.rule_id == "GL-XML-NAMESPACE"));
    assert!(matches!(
        fix_auto(xml.as_bytes(), &FixOptions::default()),
        Err(EngineError::UnsafeRewrite(_))
    ));
}

#[test]
fn unsupported_xml_envelope_metadata_is_audit_only() {
    let xml = product("").replace(
        "<title>Store</title>",
        "<title>Store</title><language>en-US</language>",
    );
    let report = audit_auto(xml.as_bytes(), &AuditOptions::default()).unwrap();
    assert!(report.structurally_complete);
    assert!(!report.rewrite_safe);
    assert!(matches!(
        fix_auto(xml.as_bytes(), &FixOptions::default()),
        Err(EngineError::UnsafeRewrite(_))
    ));

    let commented = product("").replace("<channel>", "<channel><!-- retained note -->");
    let report = audit_auto(commented.as_bytes(), &AuditOptions::default()).unwrap();
    assert!(!report.rewrite_safe);
}

#[test]
fn utf16be_xml_is_detected_before_parsing() {
    let xml = product("");
    let mut bytes = vec![0xFE, 0xFF];
    for unit in xml.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    assert_eq!(detect_format(&bytes), Some(Format::Xml));
    let report = audit_auto(&bytes, &AuditOptions::default()).unwrap();
    assert_eq!(report.product_count, 1);
}

#[test]
fn supported_flat_xml_still_fixes_and_reparses() {
    let xml = product("").replace("in_stock", "In Stock");
    let result = fix_auto(xml.as_bytes(), &FixOptions::default()).unwrap();
    assert_eq!(result.after.greenlight_score, Some(100));
    let report = audit_auto(&result.corrected_feed, &AuditOptions::default()).unwrap();
    assert!(report.structurally_complete);
    assert!(report.rewrite_safe);
}

#[test]
fn parser_limits_depth_attributes_and_field_size() {
    let deep = format!(
        "{}x{}",
        "<a>".repeat(greenlit_engine::parse::MAX_XML_DEPTH + 1),
        "</a>".repeat(greenlit_engine::parse::MAX_XML_DEPTH + 1)
    );
    assert!(matches!(
        greenlit_engine::parse::parse(deep.as_bytes(), Format::Xml),
        Err(EngineError::Limit(_))
    ));

    let attributes = (0..=greenlit_engine::parse::MAX_ATTRIBUTES_PER_ELEMENT)
        .map(|index| format!(" a{index}=\"x\""))
        .collect::<String>();
    let wide = format!("<rss{attributes}></rss>");
    assert!(matches!(
        greenlit_engine::parse::parse(wide.as_bytes(), Format::Xml),
        Err(EngineError::Limit(_))
    ));

    let huge_title = "x".repeat(greenlit_engine::parse::MAX_FIELD_BYTES + 1);
    let xml = product("").replace("A sufficiently descriptive product title", &huge_title);
    assert!(matches!(
        greenlit_engine::parse::parse(xml.as_bytes(), Format::Xml),
        Err(EngineError::Limit(_))
    ));
}
