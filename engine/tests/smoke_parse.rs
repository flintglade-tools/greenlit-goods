use greenlit_engine::parse::parse_auto;

#[test]
fn parses_rss_with_g_namespace_and_nested_shipping() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:g="http://base.google.com/ns/1.0" version="2.0">
  <channel>
    <title>My Store</title>
    <link>https://store.example/</link>
    <description>Best store</description>
    <item>
      <g:id>SKU1</g:id>
      <title>Red Shoe</title>
      <link>https://store.example/red</link>
      <g:price>49.99 USD</g:price>
      <g:additional_image_link>https://img/1.jpg</g:additional_image_link>
      <g:additional_image_link>https://img/2.jpg</g:additional_image_link>
      <g:shipping>
        <g:country>US</g:country>
        <g:price>5.00 USD</g:price>
      </g:shipping>
    </item>
    <item>
      <g:id>SKU2</g:id>
      <title>Blue Shoe</title>
    </item>
  </channel>
</rss>"#;
    let feed = parse_auto(xml.as_bytes()).expect("parse");
    assert_eq!(feed.products.len(), 2, "two items");
    assert_eq!(feed.channel.title.as_deref(), Some("My Store"));
    let p = &feed.products[0];
    assert_eq!(p.id(), Some("SKU1"));
    assert_eq!(p.get("title"), Some("Red Shoe"));
    // top-level price must NOT be clobbered by nested shipping price
    assert_eq!(
        p.get("price"),
        Some("49.99 USD"),
        "nested price must not clobber"
    );
    // multivalue join
    assert_eq!(
        p.get("additional_image_link"),
        Some("https://img/1.jpg,https://img/2.jpg")
    );
    // nested shipping flattened into the shipping attribute (best-effort)
    assert!(
        p.get("shipping").is_some(),
        "shipping captured as flattened text"
    );
}

#[test]
fn parses_csv_with_aliased_headers_and_ragged_row() {
    let csv = "ID,Title,Image Link,Price\nSKU1,Hat,https://img/h.jpg,10.00 USD\nSKU2,Scarf,https://img/s.jpg,12.00 USD,OOPS_EXTRA\n";
    let feed = parse_auto(csv.as_bytes()).expect("parse");
    assert_eq!(feed.products.len(), 2);
    assert_eq!(
        feed.products[0].get("image_link"),
        Some("https://img/h.jpg")
    );
    // extra field preserved, ragged row flagged
    assert_eq!(feed.products[1].get("extra_field_5"), Some("OOPS_EXTRA"));
    assert!(feed
        .parse_findings
        .iter()
        .any(|f| f.rule_id == "GL-CSV-RAGGED"));
}

#[test]
fn detects_tab_delimiter() {
    let tsv = "id\ttitle\tprice\nSKU1\tThing\t9.99 USD\n";
    let feed = parse_auto(tsv.as_bytes()).expect("parse");
    assert_eq!(feed.products.len(), 1);
    assert_eq!(feed.products[0].get("title"), Some("Thing"));
}
