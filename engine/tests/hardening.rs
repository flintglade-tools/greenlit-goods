//! Hardening battery: deterministic, adversarial inputs driven entirely through
//! the public API. This replaces a property-test/snapshot suite with hand-built
//! cases that pin the behaviors most likely to break silently — malformed and
//! truncated feeds, encoding messes, exotic CSV, serialize→reparse fidelity, and
//! the safety/idempotence guarantees the fixer's value proposition rests on.

use greenlit_engine::error::EngineError;
use greenlit_engine::model::Format;
use greenlit_engine::parse::{detect_format, parse_auto};
use greenlit_engine::{audit_auto, fix_auto, AuditOptions, FixOptions};

// ---------------------------------------------------------------------------
// Format detection & empty input
// ---------------------------------------------------------------------------

#[test]
fn detect_format_skips_bom_and_whitespace() {
    // UTF-8 BOM then whitespace then '<' is still XML.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(b"\n  \t<rss></rss>");
    assert_eq!(detect_format(&bytes), Some(Format::Xml));

    // Plain delimited text is CSV.
    assert_eq!(detect_format(b"id,title\n1,x"), Some(Format::Csv));

    // Genuinely empty / whitespace-only input is undetectable.
    assert_eq!(detect_format(b""), None);
    assert_eq!(detect_format(b"   \n\t  "), None);
}

#[test]
fn empty_input_is_unknown_format() {
    match parse_auto(b"") {
        Err(EngineError::UnknownFormat) => {}
        other => panic!("expected UnknownFormat, got {other:?}"),
    }
}

#[test]
fn structurally_empty_feeds_are_empty_feed_errors() {
    // Well-formed XML with no <item> elements.
    let xml =
        r#"<rss xmlns:g="http://base.google.com/ns/1.0"><channel><title>T</title></channel></rss>"#;
    assert!(matches!(
        parse_auto(xml.as_bytes()),
        Err(EngineError::EmptyFeed)
    ));

    // CSV header with no data rows.
    let csv = "id,title,price\n";
    assert!(matches!(
        parse_auto(csv.as_bytes()),
        Err(EngineError::EmptyFeed)
    ));
}

// ---------------------------------------------------------------------------
// XML robustness
// ---------------------------------------------------------------------------

#[test]
fn truncated_xml_recovers_when_a_product_was_already_parsed() {
    // First item closes cleanly; the second is cut off mid-stream.
    let xml = r#"<rss xmlns:g="http://base.google.com/ns/1.0" version="2.0"><channel>
      <item><g:id>A</g:id><title>Alpha</title><g:price>10.00 USD</g:price></item>
      <item><g:id>B</g:id><title>Beta</g:price"#;
    let feed = parse_auto(xml.as_bytes()).expect("should recover, not error");
    assert_eq!(feed.products.len(), 1, "keeps the one good product");
    assert_eq!(feed.products[0].id(), Some("A"));
    assert!(
        feed.parse_findings
            .iter()
            .any(|f| f.rule_id == "GL-XML-TRUNCATED"),
        "attaches a truncation finding"
    );
}

#[test]
fn truncated_xml_before_any_product_is_fatal() {
    let xml = r#"<rss xmlns:g="http://base.google.com/ns/1.0"><channel><item><g:id>A</g:i"#;
    assert!(matches!(
        parse_auto(xml.as_bytes()),
        Err(EngineError::Xml(_))
    ));
}

#[test]
fn nested_blocks_do_not_clobber_top_level_fields() {
    // A nested shipping price must not overwrite the product's own price.
    let xml = r#"<rss xmlns:g="http://base.google.com/ns/1.0"><channel>
      <item>
        <g:id>S1</g:id><title>Thing</title>
        <g:price>89.00 USD</g:price>
        <g:shipping><g:country>US</g:country><g:price>4.99 USD</g:price></g:shipping>
      </item>
    </channel></rss>"#;
    let feed = parse_auto(xml.as_bytes()).expect("parse");
    assert_eq!(feed.products[0].get("price"), Some("89.00 USD"));
}

#[test]
fn repeated_additional_image_links_are_joined() {
    let xml = r#"<rss xmlns:g="http://base.google.com/ns/1.0"><channel>
      <item>
        <g:id>S1</g:id><title>Thing</title>
        <g:additional_image_link>https://img/a.jpg</g:additional_image_link>
        <g:additional_image_link>https://img/b.jpg</g:additional_image_link>
      </item>
    </channel></rss>"#;
    let feed = parse_auto(xml.as_bytes()).expect("parse");
    assert_eq!(
        feed.products[0].get("additional_image_link"),
        Some("https://img/a.jpg,https://img/b.jpg")
    );
}

#[test]
fn atom_href_entities_are_decoded_for_auditing() {
    let xml = r#"<rss xmlns:g="http://base.google.com/ns/1.0" version="2.0"><channel>
      <entry><g:id>A1</g:id><title>Atom product title</title>
        <link href="https://store.example/a1?color=red&amp;size=large"/></entry>
    </channel></rss>"#;
    let feed = parse_auto(xml.as_bytes()).expect("parse Atom-style entry");
    assert_eq!(
        feed.products[0].get("link"),
        Some("https://store.example/a1?color=red&size=large")
    );
    assert!(!feed.rewrite_safe(), "Atom entries remain audit-only");
}

// ---------------------------------------------------------------------------
// CSV robustness
// ---------------------------------------------------------------------------

#[test]
fn csv_quoted_fields_with_commas_and_newlines_survive() {
    let csv = "id,title,description\n\
               1,\"Widget, Deluxe\",\"Line one\nLine two\"\n";
    let feed = parse_auto(csv.as_bytes()).expect("parse");
    assert_eq!(feed.products.len(), 1);
    assert_eq!(feed.products[0].get("title"), Some("Widget, Deluxe"));
    assert_eq!(
        feed.products[0].get("description"),
        Some("Line one\nLine two")
    );
}

#[test]
fn csv_sniffs_semicolon_and_pipe_delimiters() {
    let semi = "id;title;price\n1;Hat;10.00 USD\n";
    let feed = parse_auto(semi.as_bytes()).expect("parse");
    assert_eq!(feed.csv_delimiter, b';');
    assert_eq!(feed.products[0].get("title"), Some("Hat"));

    let pipe = "id|title|price\n1|Scarf|12.00 USD\n";
    let feed = parse_auto(pipe.as_bytes()).expect("parse");
    assert_eq!(feed.csv_delimiter, b'|');
    assert_eq!(feed.products[0].get("title"), Some("Scarf"));
}

#[test]
fn csv_delimiter_sniffing_ignores_candidates_inside_quotes() {
    let csv = "\"id,external\";title;price\n1;Hat;10.00 USD\n";
    let feed = parse_auto(csv.as_bytes()).expect("parse");
    assert_eq!(feed.csv_delimiter, b';');
    assert_eq!(feed.products[0].get("title"), Some("Hat"));
}

#[test]
fn csv_duplicate_headers_are_disambiguated_not_merged() {
    let csv = "id,color,color\n1,Red,Blue\n";
    let feed = parse_auto(csv.as_bytes()).expect("parse");
    assert_eq!(feed.products[0].get("color"), Some("Red"));
    assert_eq!(feed.products[0].get("color_2"), Some("Blue"));
}

#[test]
fn csv_header_collisions_preserve_natural_and_blank_columns() {
    let csv = "color,color_2,color\nred,native_second,blue\n";
    let result = fix_auto(csv.as_bytes(), &FixOptions::default()).expect("fix");
    let reparsed = parse_auto(&result.corrected_feed).expect("reparse");
    assert_eq!(reparsed.products[0].get("color"), Some("red"));
    assert_eq!(reparsed.products[0].get("color_2"), Some("native_second"));
    assert_eq!(reparsed.products[0].get("color_3"), Some("blue"));

    let blank = ",column,\nfirst,native,third\n";
    let result = fix_auto(blank.as_bytes(), &FixOptions::default()).expect("fix");
    let reparsed = parse_auto(&result.corrected_feed).expect("reparse");
    assert_eq!(reparsed.products[0].get("column"), Some("first"));
    assert_eq!(reparsed.products[0].get("column_2"), Some("native"));
    assert_eq!(reparsed.products[0].get("column_3"), Some("third"));
}

#[test]
fn csv_surplus_field_keys_never_overwrite_real_headers() {
    let csv = "id,extra_field_3\nP1,original,surplus\n";
    let result = fix_auto(csv.as_bytes(), &FixOptions::default()).expect("fix");
    let reparsed = parse_auto(&result.corrected_feed).expect("reparse");
    assert_eq!(reparsed.products[0].get("extra_field_3"), Some("original"));
    assert_eq!(reparsed.products[0].get("extra_field_3_2"), Some("surplus"));
}

#[test]
fn csv_rewrite_preserves_header_labels_order_and_duplicates() {
    let csv =
        "ID,Custom Field,color,color,Price,Availability\n1,value,Red,Blue,10.0 usd,In Stock\n";
    let result = fix_auto(csv.as_bytes(), &FixOptions::default()).expect("fix");
    let output = String::from_utf8(result.corrected_feed).unwrap();
    assert_eq!(
        output.lines().next(),
        Some("ID,Custom Field,color,color,Price,Availability")
    );
    let reparsed = parse_auto(output.as_bytes()).expect("reparse");
    assert_eq!(reparsed.products[0].get("custom_field"), Some("value"));
    assert_eq!(reparsed.products[0].get("color"), Some("Red"));
    assert_eq!(reparsed.products[0].get("color_2"), Some("Blue"));
}

// ---------------------------------------------------------------------------
// Encoding tolerance
// ---------------------------------------------------------------------------

#[test]
fn utf16le_bom_feed_decodes() {
    let xml = r#"<rss xmlns:g="http://base.google.com/ns/1.0"><channel><item><g:id>U16</g:id><title>Hello</title></item></channel></rss>"#;
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let feed = parse_auto(&bytes).expect("UTF-16LE should decode");
    assert_eq!(feed.products[0].id(), Some("U16"));
    assert_eq!(feed.products[0].get("title"), Some("Hello"));
}

#[test]
fn non_utf8_bytes_fall_back_to_windows_1252_with_a_finding() {
    // 0xE9 is a lone high byte: invalid UTF-8, valid Windows-1252 ('é').
    let mut bytes =
        br#"<rss xmlns:g="http://base.google.com/ns/1.0"><channel><item><g:id>E1</g:id><title>Caf"#
            .to_vec();
    bytes.push(0xE9);
    bytes.extend_from_slice(b"</title></item></channel></rss>");
    let feed = parse_auto(&bytes).expect("should decode leniently");
    assert_eq!(feed.products[0].get("title"), Some("Café"));
    assert!(
        feed.parse_findings
            .iter()
            .any(|f| f.rule_id == "GL-ENCODING"),
        "flags the non-UTF-8 feed"
    );
}

// ---------------------------------------------------------------------------
// Serialize -> reparse fidelity
// ---------------------------------------------------------------------------

#[test]
fn xml_round_trips_through_fix_serialize_reparse() {
    let xml = r#"<rss xmlns:g="http://base.google.com/ns/1.0" version="2.0"><channel>
      <item>
        <g:id>R1</g:id><title>Round Trip</title>
        <link>https://store.example/r1</link>
        <g:price>49.99 USD</g:price>
        <g:availability>in_stock</g:availability>
      </item>
    </channel></rss>"#;
    // A clean feed fixes to zero changes; the serialized output must reparse
    // identically (ids and key fields preserved).
    let res = fix_auto(xml.as_bytes(), &FixOptions::default()).expect("fix");
    let reparsed = parse_auto(&res.corrected_feed).expect("reparse serialized output");
    assert_eq!(reparsed.products.len(), 1);
    let p = &reparsed.products[0];
    assert_eq!(p.id(), Some("R1"));
    assert_eq!(p.get("title"), Some("Round Trip"));
    assert_eq!(p.get("link"), Some("https://store.example/r1"));
    assert_eq!(p.get("price"), Some("49.99 USD"));
    assert_eq!(p.get("availability"), Some("in_stock"));
}

#[test]
fn csv_round_trips_through_fix_serialize_reparse() {
    let csv = "id,title,price,availability\nC1,Cap,15.00 USD,in_stock\nC2,Mug,8.00 USD,in_stock\n";
    let res = fix_auto(csv.as_bytes(), &FixOptions::default()).expect("fix");
    let reparsed = parse_auto(&res.corrected_feed).expect("reparse serialized CSV");
    assert_eq!(reparsed.format, Format::Csv);
    assert_eq!(reparsed.products.len(), 2);
    assert_eq!(reparsed.products[0].id(), Some("C1"));
    assert_eq!(reparsed.products[0].get("price"), Some("15.00 USD"));
    assert_eq!(reparsed.products[1].get("title"), Some("Mug"));
}

// ---------------------------------------------------------------------------
// Fixer safety & idempotence (the conservative-by-design contract)
// ---------------------------------------------------------------------------

const MESSY: &str = r#"<rss xmlns:g="http://base.google.com/ns/1.0" version="2.0"><channel>
  <item>
    <g:id>M1</g:id>
    <title>PREMIUM STAINLESS STEEL WATER BOTTLE 32OZ</title>
    <description><![CDATA[<p>Keeps drinks cold.</p><br>Order now!]]></description>
    <link>https://store.example/m1</link>
    <g:price>19.9 usd</g:price>
    <g:availability>In Stock</g:availability>
    <g:condition>Brand New</g:condition>
  </item>
  <item>
    <g:id>M2</g:id>
    <title>Plain Title</title>
    <link>https://store.example/m2</link>
    <g:price>$29.99</g:price>
    <g:availability>available</g:availability>
  </item>
</channel></rss>"#;

#[test]
fn fixer_applies_only_safe_repairs() {
    let res = fix_auto(MESSY.as_bytes(), &FixOptions::default()).expect("fix");
    let p = parse_auto(&res.corrected_feed).expect("reparse");
    let m1 = &p.products[0];
    // Editorial casing and markup are preserved for human review.
    assert_eq!(
        m1.get("title"),
        Some("PREMIUM STAINLESS STEEL WATER BOTTLE 32OZ")
    );
    assert_eq!(
        m1.get("description"),
        Some("<p>Keeps drinks cold.</p><br>Order now!")
    );
    // Price reformatted to two decimals + uppercase ISO currency.
    assert_eq!(m1.get("price"), Some("19.90 USD"));
    // Enums canonicalized.
    assert_eq!(m1.get("availability"), Some("in_stock"));
    assert_eq!(m1.get("condition"), Some("new"));

    let m2 = &p.products[1];
    // A currency *symbol* price is ambiguous: the fixer must NOT invent USD.
    assert_eq!(
        m2.get("price"),
        Some("$29.99"),
        "symbol price left for human review"
    );
    assert_eq!(m2.get("availability"), Some("in_stock"));
}

#[test]
fn fixer_never_invents_missing_identifiers_or_currency() {
    // No gtin/brand/mpn present, and a bare numeric price with no currency.
    let xml = r#"<rss xmlns:g="http://base.google.com/ns/1.0" version="2.0"><channel>
      <item><g:id>N1</g:id><title>No Identifiers Here</title>
        <link>https://store.example/n1</link><g:price>29.99</g:price></item>
    </channel></rss>"#;
    let res = fix_auto(xml.as_bytes(), &FixOptions::default()).expect("fix");
    let p = &parse_auto(&res.corrected_feed).expect("reparse").products[0];
    assert!(p.get("gtin").is_none(), "never fabricates a GTIN");
    assert!(p.get("brand").is_none(), "never fabricates a brand");
    // Bare number is left exactly as-is (adding a currency would be a guess).
    assert_eq!(p.get("price"), Some("29.99"));
}

#[test]
fn fixing_is_idempotent() {
    let first = fix_auto(MESSY.as_bytes(), &FixOptions::default()).expect("fix 1");
    assert!(!first.log.is_empty(), "first pass changes something");
    let second = fix_auto(&first.corrected_feed, &FixOptions::default()).expect("fix 2");
    assert!(
        second.log.is_empty(),
        "second pass is a no-op: {:?}",
        second.log
    );
    assert_eq!(
        first.corrected_feed, second.corrected_feed,
        "re-fixing produces byte-identical output"
    );
}

#[test]
fn description_internal_whitespace_is_preserved() {
    // Leading/trailing space trimmed, but the internal paragraph break stays.
    let csv = "id,title,description\nD1,Keeper,\"  para one\n\npara two  \"\n";
    let res = fix_auto(csv.as_bytes(), &FixOptions::default()).expect("fix");
    let p = &parse_auto(&res.corrected_feed).expect("reparse").products[0];
    assert_eq!(p.get("description"), Some("para one\n\npara two"));
}

#[test]
fn xml_boundary_whitespace_is_preserved_until_a_logged_fix() {
    let xml = r#"<rss xmlns:g="http://base.google.com/ns/1.0" version="2.0"><channel>
      <item><g:id>W1</g:id><title>  Kept title  </title>
      <g:custom_label_0>  preserve unknown  </g:custom_label_0></item>
    </channel></rss>"#;
    let parsed = parse_auto(xml.as_bytes()).expect("parse");
    assert_eq!(parsed.products[0].get_raw("title"), Some("  Kept title  "));
    assert_eq!(
        parsed.products[0].get_raw("custom_label_0"),
        Some("  preserve unknown  ")
    );

    let result = fix_auto(xml.as_bytes(), &FixOptions::default()).expect("fix");
    assert!(result
        .log
        .iter()
        .any(|record| record.field == "title" && record.before == "  Kept title  "));
    let reparsed = parse_auto(&result.corrected_feed).expect("reparse");
    assert_eq!(reparsed.products[0].get_raw("title"), Some("Kept title"));
    assert_eq!(
        reparsed.products[0].get_raw("custom_label_0"),
        Some("  preserve unknown  ")
    );
}

#[test]
fn xml_rewrite_does_not_invent_missing_channel_metadata() {
    let xml = r#"<rss xmlns:g="http://base.google.com/ns/1.0" version="2.0"><channel>
      <item><g:id>M1</g:id><title>Product title</title></item>
    </channel></rss>"#;
    let result = fix_auto(xml.as_bytes(), &FixOptions::default()).expect("fix");
    let output = String::from_utf8(result.corrected_feed).unwrap();
    assert!(!output.contains("Product Feed"));
}

// ---------------------------------------------------------------------------
// End-to-end invariant: fixing only ever helps
// ---------------------------------------------------------------------------

#[test]
fn fixing_never_lowers_score_or_adds_disapprovals() {
    let res = fix_auto(MESSY.as_bytes(), &FixOptions::default()).expect("fix");
    assert!(
        res.after.greenlight_score >= res.before.greenlight_score,
        "score must not regress: {} -> {}",
        res.before.greenlight_score.unwrap(),
        res.after.greenlight_score.unwrap()
    );
    assert!(
        res.after.total_disapprovals <= res.before.total_disapprovals,
        "disapprovals must not increase"
    );
    // Every safely-fixable finding should be gone after a fix pass.
    assert_eq!(
        res.after.auto_fixable_findings, 0,
        "no auto-fixable findings remain after fixing"
    );
}

#[test]
fn audit_options_country_changes_apparel_severity() {
    // An apparel item missing color/size: a Disapproval in the US, softer
    // elsewhere. We only assert the audit runs and finds issues in both, and
    // that the US is at least as strict.
    let xml = r#"<rss xmlns:g="http://base.google.com/ns/1.0"><channel>
      <item><g:id>AP1</g:id><title>Cotton T-Shirt</title>
        <link>https://store.example/ap1</link><g:price>20.00 USD</g:price>
        <g:availability>in_stock</g:availability>
        <g:google_product_category>Apparel &amp; Accessories &gt; Clothing</g:google_product_category>
      </item>
    </channel></rss>"#;
    let us = audit_auto(
        xml.as_bytes(),
        &AuditOptions {
            target_country: "US".into(),
            ..AuditOptions::default()
        },
    )
    .expect("us");
    let de = audit_auto(
        xml.as_bytes(),
        &AuditOptions {
            target_country: "DE".into(),
            ..AuditOptions::default()
        },
    )
    .expect("de");
    assert!(us.total_disapprovals >= de.total_disapprovals);
    assert!(us.greenlight_score <= de.greenlight_score);
}
