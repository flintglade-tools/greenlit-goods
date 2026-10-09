use greenlit_engine::error::EngineError;
use greenlit_engine::parse::{parse_auto, MAX_CSV_COLUMNS, MAX_FIELD_BYTES};
use greenlit_engine::serialize::serialize;

#[test]
fn hostile_header_width_is_rejected_before_disambiguation() {
    let input = format!("{}\nvalue\n", vec!["id"; MAX_CSV_COLUMNS + 1].join(","));
    assert!(matches!(
        parse_auto(input.as_bytes()),
        Err(EngineError::Limit(_))
    ));
}

#[test]
fn duplicate_headers_preserve_reserved_source_names() {
    let feed = parse_auto(b"id,id,id_2,id\na,b,c,d\n").unwrap();
    assert_eq!(feed.products[0].get_raw("id"), Some("a"));
    assert_eq!(feed.products[0].get_raw("id_2"), Some("c"));
    assert_eq!(feed.products[0].get_raw("id_3"), Some("b"));
    assert_eq!(feed.products[0].get_raw("id_4"), Some("d"));
}

#[test]
fn sparse_rows_cannot_expand_into_an_unbounded_rectangle() {
    let input = format!(
        "{}\n{}",
        (0..MAX_CSV_COLUMNS)
            .map(|n| format!("field_{n}"))
            .collect::<Vec<_>>()
            .join(","),
        "a\n".repeat(4100)
    );
    let feed = parse_auto(input.as_bytes()).unwrap();
    assert!(matches!(serialize(&feed), Err(EngineError::Limit(_))));
}

#[test]
fn repeated_xml_values_enforce_the_combined_field_limit() {
    let value = "x".repeat(MAX_FIELD_BYTES / 2 + 1);
    let input = xml(&format!(
        "<g:product_type>{value}</g:product_type><g:product_type>{value}</g:product_type>"
    ));
    assert!(matches!(
        parse_auto(input.as_bytes()),
        Err(EngineError::Limit(_))
    ));
}

#[test]
fn three_plain_multivalue_elements_round_trip_without_false_ambiguity() {
    let feed = parse_auto(xml("<g:product_type>A</g:product_type><g:product_type>B</g:product_type><g:product_type>C</g:product_type>").as_bytes()).unwrap();
    let output = serialize(&feed).unwrap();
    let reparsed = parse_auto(&output).unwrap();
    assert_eq!(reparsed.products[0].get_raw("product_type"), Some("A,B,C"));
}

#[test]
fn a_source_comma_is_not_silently_split_on_rewrite() {
    let feed =
        parse_auto(xml("<g:product_type>Home, kitchen</g:product_type>").as_bytes()).unwrap();
    assert!(matches!(
        serialize(&feed),
        Err(EngineError::UnsafeRewrite(_))
    ));
}

#[test]
fn unfinished_markup_is_a_linear_scan() {
    let input = "<a".repeat(500_000);
    let started = std::time::Instant::now();
    assert!(!greenlit_engine::text::has_html(&input));
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(greenlit_engine::text::has_html(&format!("{input}>")));
}

fn xml(fields: &str) -> String {
    format!("<rss xmlns:g=\"http://base.google.com/ns/1.0\" version=\"2.0\"><channel><item><g:id>A</g:id>{fields}</item></channel></rss>")
}
