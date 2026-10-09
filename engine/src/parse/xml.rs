//! Streaming XML parser for RSS 2.0 feeds using the Google base namespace
//! (`g:` attributes), with light tolerance for Atom (`entry`/`href` links).
//!
//! Two non-obvious decisions:
//!
//! * **Nested elements are flattened, not recursed.** A product attribute is any
//!   element whose direct parent is `<item>`. If such an element itself has
//!   children (e.g. a nested `<g:shipping>` block), we concatenate the
//!   descendant text into the attribute value rather than emitting the children
//!   as their own attributes. That prevents a child `<g:price>` inside shipping
//!   from clobbering the product's top-level `price`. Deep shipping/tax modeling
//!   is intentionally out of scope for v1 and noted as a limitation.
//!
//! * **Truncation is recoverable.** If the XML breaks partway through but we have
//!   already parsed at least one product, we keep what we have and attach a
//!   finding, rather than throwing the whole feed away.

use quick_xml::escape::resolve_xml_entity;
use quick_xml::events::{BytesCData, BytesRef, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::{encoding::Decoder, XmlVersion};

use crate::error::{EngineError, Result};
use crate::finding::{Finding, Severity};
use crate::model::{canonical_attr, Feed, Format, Product};
use crate::parse::decode_lenient;
use crate::parse::{MAX_ATTRIBUTES_PER_ELEMENT, MAX_FIELD_BYTES, MAX_PRODUCTS, MAX_XML_DEPTH};

/// Attributes for which repeated elements are meaningful and should be joined
/// rather than having the first occurrence win.
const MULTIVALUE: &[&str] = &["additional_image_link", "product_type"];
const GOOGLE_PRODUCT_NAMESPACE: &[u8] = b"http://base.google.com/ns/1.0";

pub fn parse_xml(bytes: &[u8]) -> Result<Feed> {
    let (text, enc_finding) = decode_lenient(bytes);
    let mut reader = Reader::from_str(&text);

    let mut feed = Feed::new(Format::Xml);
    if let Some(f) = enc_finding {
        feed.block_rewrite("source text required a best-effort encoding recovery");
        feed.parse_findings.push(f);
    }

    let mut buf = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut current: Option<Product> = None;

    // Attribute-capture state (element that is a direct child of <item>).
    let mut cap_attr: Option<String> = None;
    let mut cap_buf = String::new();
    let mut cap_depth = 0usize;
    let mut cap_href: Option<String> = None;

    // Channel-meta capture state (direct child of <channel>, outside items).
    let mut chan_field: Option<&'static str> = None;
    let mut chan_buf = String::new();
    let mut chan_depth = 0usize;

    let mut record_no = 0u64;
    let mut seen_rss = false;
    let mut seen_channel = false;
    let mut document_started = false;

    loop {
        let event = reader.read_event_into(&mut buf);
        let declaration_is_misplaced = matches!(&event, Ok(Event::Decl(_))) && document_started;
        if !matches!(&event, Ok(Event::Eof)) {
            document_started = true;
        }

        match event {
            Ok(Event::Start(e)) => {
                enforce_element_limits(&e, stack.len())?;
                let raw_name = qname(&e);
                let name = canonical_attr(&raw_name);
                let parent = stack.last().map(String::as_str);

                if stack.is_empty() && name == "rss" {
                    if seen_rss {
                        feed.block_rewrite("multiple RSS root elements cannot be rewritten safely");
                    }
                    seen_rss = true;
                    if !has_google_product_namespace(&e) {
                        feed.mark_incomplete(
                            "RSS root is missing the required Google product namespace declaration",
                        );
                        feed.parse_findings.push(
                            Finding::new(
                                "GL-XML-NAMESPACE",
                                Severity::Disapproval,
                                "RSS feed is missing xmlns:g=\"http://base.google.com/ns/1.0\".",
                            )
                            .detail(
                                "Google product attributes require the documented Google base namespace.",
                            )
                            .structural(),
                        );
                    }
                    if has_unsupported_rss_attributes(&e) {
                        feed.block_rewrite(
                            "attributes on the RSS root are outside the supported rewrite envelope",
                        );
                    }
                    if !has_rss_version_2(&e) {
                        feed.block_rewrite(
                            "RSS root is missing version=\"2.0\"; rewriting would invent it",
                        );
                    }
                } else if name == "channel" && parent == Some("rss") {
                    if seen_channel {
                        feed.block_rewrite(
                            "multiple <channel> elements cannot be rewritten safely",
                        );
                    }
                    seen_channel = true;
                    if e.attributes().next().is_some() {
                        feed.block_rewrite(
                            "attributes on <channel> are outside the supported rewrite envelope",
                        );
                    }
                } else if name == "item" || name == "entry" {
                    if current.is_some() {
                        return Err(EngineError::Xml(
                            "nested product item/entry elements are not supported".into(),
                        ));
                    }
                    if name == "entry" {
                        feed.block_rewrite(
                            "Atom <entry> input is audit-only; rewriting emits RSS 2.0",
                        );
                    }
                    if name == "item" && parent != Some("channel") {
                        feed.block_rewrite(
                            "RSS <item> elements must be direct children of <channel>",
                        );
                    }
                    if name == "item" && e.attributes().next().is_some() {
                        feed.block_rewrite(
                            "attributes on <item> are outside the supported rewrite envelope",
                        );
                    } else if name == "entry" && has_non_href_attributes(&e) {
                        feed.block_rewrite(format!(
                            "attributes on <{raw_name}> are not represented for rewriting"
                        ));
                    }
                    record_no += 1;
                    let mut p = Product::new();
                    p.source_record = Some(record_no);
                    current = Some(p);
                } else if current.is_some()
                    && cap_attr.is_none()
                    && matches!(parent, Some("item") | Some("entry"))
                {
                    // Begin capturing a product attribute.
                    if let Some(reason) = unsupported_item_child(&raw_name, &name) {
                        feed.block_rewrite(reason);
                    }
                    let atom_href_link = parent == Some("entry") && name == "link";
                    let unsupported_attributes = if atom_href_link {
                        has_non_href_attributes(&e)
                    } else {
                        e.attributes().next().is_some()
                    };
                    if unsupported_attributes {
                        feed.block_rewrite(format!(
                            "attributes on product element <{raw_name}> are not represented"
                        ));
                    }
                    cap_attr = Some(name.clone());
                    cap_buf.clear();
                    cap_depth = stack.len();
                    cap_href = if atom_href_link {
                        get_href(&e, reader.decoder())?
                    } else {
                        None
                    };
                } else if current.is_some() && cap_attr.is_some() {
                    feed.block_rewrite(format!(
                        "nested product structure inside '{}' is audit-only",
                        cap_attr.as_deref().unwrap_or("unknown")
                    ));
                } else if current.is_none() && chan_field.is_none() && parent == Some("channel") {
                    if e.attributes().next().is_some() {
                        feed.block_rewrite(format!(
                            "attributes on channel element <{raw_name}> are not represented"
                        ));
                    }
                    if let Some(f) = channel_field(&name) {
                        let repeated = match f {
                            "title" => feed.channel.title.is_some(),
                            "link" => feed.channel.link.is_some(),
                            "description" => feed.channel.description.is_some(),
                            _ => false,
                        };
                        if repeated {
                            feed.block_rewrite(format!(
                                "repeated channel element <{raw_name}> cannot be rewritten safely"
                            ));
                        }
                        chan_field = Some(f);
                        chan_buf.clear();
                        chan_depth = stack.len();
                    } else {
                        feed.block_rewrite(format!(
                            "channel element <{raw_name}> is outside the supported rewrite envelope"
                        ));
                    }
                } else if current.is_none() {
                    feed.block_rewrite(format!(
                        "XML envelope element <{raw_name}> is not represented for rewriting"
                    ));
                }

                stack.push(name);
            }

            Ok(Event::Empty(e)) => {
                // Self-closing element: behaves as start+end with no text body.
                enforce_element_limits(&e, stack.len())?;
                let raw_name = qname(&e);
                let name = canonical_attr(&raw_name);
                let parent = stack.last().map(String::as_str);
                if let (Some(product), Some("item" | "entry")) = (current.as_mut(), parent) {
                    if let Some(reason) = unsupported_item_child(&raw_name, &name) {
                        feed.block_rewrite(reason);
                    }
                    let atom_href_link = parent == Some("entry") && name == "link";
                    let unsupported_attributes = if atom_href_link {
                        has_non_href_attributes(&e)
                    } else {
                        e.attributes().next().is_some()
                    };
                    if unsupported_attributes {
                        feed.block_rewrite(format!(
                            "attributes on product element <{raw_name}> are not represented"
                        ));
                    }
                    // Atom links carry the URL in an href attribute.
                    if atom_href_link {
                        if let Some(href) = get_href(&e, reader.decoder())? {
                            if let Some(reason) = store_attr(product, &name, href)? {
                                feed.block_rewrite(reason);
                            }
                        } else {
                            feed.block_rewrite(format!(
                                "empty product element <{raw_name}/> is omitted during rewriting"
                            ));
                        }
                    } else {
                        feed.block_rewrite(format!(
                            "empty product element <{raw_name}/> is omitted during rewriting"
                        ));
                    }
                } else if current.is_some() && cap_attr.is_some() {
                    feed.block_rewrite(format!(
                        "nested product structure inside '{}' is audit-only",
                        cap_attr.as_deref().unwrap_or("unknown")
                    ));
                } else {
                    feed.block_rewrite(format!(
                        "empty XML envelope element <{raw_name}/> is not represented for rewriting"
                    ));
                }
            }

            Ok(Event::Text(e)) => {
                let t = match decode_text(&e) {
                    Ok(text) => text,
                    Err(error) => {
                        feed.mark_incomplete("an XML entity could not be decoded");
                        feed.parse_findings.push(
                            Finding::new(
                                "GL-XML-ENTITY",
                                Severity::AtRisk,
                                "XML text contained an invalid entity and is incomplete.",
                            )
                            .detail(error.to_string())
                            .structural(),
                        );
                        String::new()
                    }
                };
                if cap_attr.is_some() {
                    cap_buf.push_str(&t);
                    enforce_field_limit(&cap_buf)?;
                } else if chan_field.is_some() {
                    chan_buf.push_str(&t);
                    enforce_field_limit(&chan_buf)?;
                } else if !t.trim().is_empty() {
                    feed.block_rewrite(
                        "non-whitespace XML envelope text is not represented for rewriting",
                    );
                }
            }

            Ok(Event::GeneralRef(e)) => {
                let resolved = resolve_reference(&e);
                match resolved {
                    Ok(text) => {
                        if cap_attr.is_some() {
                            cap_buf.push_str(&text);
                            enforce_field_limit(&cap_buf)?;
                        } else if chan_field.is_some() {
                            chan_buf.push_str(&text);
                            enforce_field_limit(&chan_buf)?;
                        } else if !text.trim().is_empty() {
                            feed.block_rewrite(
                                "non-whitespace XML envelope text is not represented for rewriting",
                            );
                        }
                    }
                    Err(error) => {
                        feed.mark_incomplete("an XML entity could not be decoded");
                        feed.parse_findings.push(
                            Finding::new(
                                "GL-XML-ENTITY",
                                Severity::AtRisk,
                                "XML text contained an invalid entity and is incomplete.",
                            )
                            .detail(error)
                            .structural(),
                        );
                    }
                }
            }

            Ok(Event::CData(e)) => {
                let t = match decode_cdata(&e) {
                    Ok(text) => text,
                    Err(error) => {
                        feed.mark_incomplete("XML CDATA contained invalid XML 1.0 text");
                        feed.parse_findings.push(
                            Finding::new(
                                "GL-XML-CDATA",
                                Severity::AtRisk,
                                "XML CDATA contained invalid text and is incomplete.",
                            )
                            .detail(error)
                            .structural(),
                        );
                        String::new()
                    }
                };
                if cap_attr.is_some() {
                    cap_buf.push_str(&t);
                    enforce_field_limit(&cap_buf)?;
                } else if chan_field.is_some() {
                    chan_buf.push_str(&t);
                    enforce_field_limit(&chan_buf)?;
                } else if !t.trim().is_empty() {
                    feed.block_rewrite(
                        "non-whitespace XML envelope text is not represented for rewriting",
                    );
                }
            }

            Ok(Event::End(_)) => {
                stack.pop();

                // Finalize an attribute capture once we close back to its level.
                if cap_attr.is_some() && stack.len() == cap_depth {
                    let attr = cap_attr.take().unwrap();
                    let mut value = std::mem::take(&mut cap_buf);
                    if value.trim().is_empty() {
                        if let Some(h) = cap_href.take() {
                            value = h;
                        }
                    }
                    cap_href = None;
                    if let Some(p) = current.as_mut() {
                        if let Some(reason) = store_attr(p, &attr, value)? {
                            feed.block_rewrite(reason);
                        }
                    }
                }

                // Finalize channel-meta capture.
                if chan_field.is_some() && stack.len() == chan_depth {
                    let field = chan_field.take().unwrap();
                    let value = std::mem::take(&mut chan_buf);
                    match field {
                        "title" if feed.channel.title.is_none() => feed.channel.title = Some(value),
                        "link" if feed.channel.link.is_none() => feed.channel.link = Some(value),
                        "description" if feed.channel.description.is_none() => {
                            feed.channel.description = Some(value)
                        }
                        _ => {}
                    }
                }

                // Closing an item commits the product.
                let just_closed = stack_was_item(&stack, current.is_some());
                if just_closed {
                    if let Some(p) = current.take() {
                        if !p.is_empty() {
                            if feed.products.len() >= MAX_PRODUCTS {
                                return Err(EngineError::Limit(format!(
                                    "product count exceeds {MAX_PRODUCTS}"
                                )));
                            }
                            feed.products.push(p);
                        }
                    }
                }
            }

            Ok(Event::Eof) => break,

            Ok(Event::Decl(declaration)) => {
                if declaration_is_misplaced {
                    feed.block_rewrite(
                        "XML declarations must appear once, before all document content",
                    );
                }
                let version = declaration.version().map_err(|error| {
                    EngineError::Xml(format!("invalid XML declaration: {error}"))
                })?;
                if version.as_ref() != b"1.0" {
                    feed.block_rewrite("only XML 1.0 declarations can be rewritten safely");
                }
                if let Some(encoding) = declaration.encoding() {
                    let encoding = encoding.map_err(|error| {
                        EngineError::Xml(format!("invalid XML declaration encoding: {error}"))
                    })?;
                    if !encoding.as_ref().eq_ignore_ascii_case(b"UTF-8") {
                        feed.block_rewrite(
                            "non-UTF-8 XML declarations cannot be preserved during rewriting",
                        );
                    }
                }
                if let Some(standalone) = declaration.standalone() {
                    standalone.map_err(|error| {
                        EngineError::Xml(format!("invalid XML standalone declaration: {error}"))
                    })?;
                    feed.block_rewrite(
                        "XML standalone declarations are not preserved during rewriting",
                    );
                }
            }

            Ok(Event::Comment(_)) => {
                feed.block_rewrite("XML comments are not represented for rewriting");
            }

            Ok(Event::PI(_)) => {
                feed.block_rewrite("XML processing instructions are not represented for rewriting");
            }

            Ok(Event::DocType(_)) => {
                feed.block_rewrite(
                    "XML document type declarations are not represented for rewriting",
                );
            }

            Err(e) => {
                // Recover if we already have data; otherwise it's fatal.
                if feed.products.is_empty() {
                    return Err(EngineError::Xml(format!(
                        "{e} (at byte {})",
                        reader.buffer_position()
                    )));
                }
                feed.parse_findings.push(
                    Finding::new(
                        "GL-XML-TRUNCATED",
                        Severity::AtRisk,
                        format!(
                            "The XML feed could not be fully parsed and was truncated after \
                             {} product(s).",
                            feed.products.len()
                        ),
                    )
                    .detail(format!(
                        "Parser stopped at byte {}: {e}",
                        reader.buffer_position()
                    ))
                    .structural(),
                );
                feed.mark_incomplete("XML parsing stopped before the end of the feed");
                break;
            }
        }
        buf.clear();
    }

    if feed.products.is_empty() {
        return Err(EngineError::EmptyFeed);
    }
    if !seen_rss {
        feed.block_rewrite("an RSS root element is required for rewriting");
    }
    if !seen_channel {
        feed.block_rewrite("exactly one RSS <channel> element is required for rewriting");
    }
    Ok(feed)
}

/// We need to know if the `End` we just processed closed an `<item>`/`<entry>`.
/// After `stack.pop()`, if `current` is still set and the stack no longer
/// contains an item/entry ancestor, the popped element *was* the item.
fn stack_was_item(stack: &[String], have_current: bool) -> bool {
    have_current && !stack.iter().any(|s| s == "item" || s == "entry")
}

/// Map a channel-level element name to the metadata slot it fills.
fn channel_field(name: &str) -> Option<&'static str> {
    match name {
        "title" => Some("title"),
        "link" => Some("link"),
        "description" => Some("description"),
        _ => None,
    }
}

/// Store an attribute, honoring multivalue join semantics.
fn store_attr(p: &mut Product, attr: &str, value: String) -> Result<Option<String>> {
    if MULTIVALUE.contains(&attr) {
        // Only source commas are ambiguous. Commas inserted between previous
        // values do not make a third repeated element unsafe to preserve.
        let ambiguous = value
            .contains(',')
            .then(|| format!("'{attr}' values contain commas and cannot be split safely"));
        if let Some(existing) = p.get_raw_mut(attr) {
            if existing.len().saturating_add(value.len()).saturating_add(1) > MAX_FIELD_BYTES {
                return Err(EngineError::Limit(format!(
                    "combined XML field exceeds {MAX_FIELD_BYTES} bytes"
                )));
            }
            existing.push(',');
            existing.push_str(&value);
        } else {
            p.set(attr, value);
        }
        return Ok(ambiguous);
    }
    if p.has(attr) {
        return Ok(Some(format!(
            "repeated scalar product element '{attr}' would be discarded"
        )));
    }
    // First occurrence wins for scalar attributes.
    p.set_if_absent(attr, value);
    Ok(None)
}

/// The fully-qualified element name as a lossy string (e.g. `g:price`).
fn qname(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).into_owned()
}

/// Read an element's `href` attribute, if any (used for Atom links).
fn get_href(e: &BytesStart, decoder: Decoder) -> Result<Option<String>> {
    // Every Start/Empty event is validated by `enforce_element_limits` before
    // these helpers run, so an attribute error here would violate that invariant.
    for attr in e
        .attributes()
        .map(|attr| attr.expect("XML attributes were validated before inspection"))
    {
        if attr.key.as_ref() == b"href" {
            let value = attr
                .decoded_and_normalized_value(XmlVersion::Implicit1_0, decoder)
                .map_err(|error| EngineError::Xml(format!("invalid href attribute: {error}")))?
                .into_owned();
            return validate_xml10_text(value)
                .map(Some)
                .map_err(|error| EngineError::Xml(format!("invalid href attribute: {error}")));
        }
    }
    Ok(None)
}

fn has_non_href_attributes(e: &BytesStart) -> bool {
    e.attributes()
        .map(|attr| attr.expect("XML attributes were validated before inspection"))
        .any(|attr| attr.key.as_ref() != b"href")
}

fn has_google_product_namespace(e: &BytesStart) -> bool {
    e.attributes()
        .map(|attr| attr.expect("XML attributes were validated before inspection"))
        .any(|attr| {
            attr.key.as_ref() == b"xmlns:g" && attr.value.as_ref() == GOOGLE_PRODUCT_NAMESPACE
        })
}

fn has_unsupported_rss_attributes(e: &BytesStart) -> bool {
    e.attributes()
        .map(|attr| attr.expect("XML attributes were validated before inspection"))
        .any(|attr| {
            let key = attr.key.as_ref();
            key != b"xmlns:g" && !(key == b"version" && attr.value.as_ref() == b"2.0")
        })
}

fn has_rss_version_2(e: &BytesStart) -> bool {
    e.attributes()
        .map(|attr| attr.expect("XML attributes were validated before inspection"))
        .any(|attr| attr.key.as_ref() == b"version" && attr.value.as_ref() == b"2.0")
}

fn unsupported_item_child(raw_name: &str, canonical_name: &str) -> Option<String> {
    if let Some((prefix, _)) = raw_name.split_once(':') {
        if prefix != "g" {
            return Some(format!(
                "custom namespace element <{raw_name}> is not represented for rewriting"
            ));
        }
    } else if !["title", "link", "description"].contains(&canonical_name) {
        return Some(format!(
            "unprefixed product element <{raw_name}> would change namespace on rewrite"
        ));
    }
    None
}

fn decode_text(text: &BytesText<'_>) -> std::result::Result<String, String> {
    let value = text
        .xml10_content()
        .map(|value| value.into_owned())
        .map_err(|error| error.to_string())?;
    validate_xml10_text(value)
}

fn decode_cdata(cdata: &BytesCData<'_>) -> std::result::Result<String, String> {
    let value = cdata
        .xml10_content()
        .map(|value| value.into_owned())
        .map_err(|error| error.to_string())?;
    validate_xml10_text(value)
}

fn validate_xml10_text(value: String) -> std::result::Result<String, String> {
    if let Some(character) = value.chars().find(|character| {
        !matches!(
            *character,
            '\u{9}'
                | '\u{A}'
                | '\u{D}'
                | '\u{20}'..='\u{D7FF}'
                | '\u{E000}'..='\u{FFFD}'
                | '\u{10000}'..='\u{10FFFF}'
        )
    }) {
        return Err(format!(
            "invalid XML 1.0 character U+{:04X}",
            character as u32
        ));
    }
    Ok(value)
}

fn resolve_reference(reference: &BytesRef<'_>) -> std::result::Result<String, String> {
    if let Some(ch) = reference
        .resolve_char_ref()
        .map_err(|error| error.to_string())?
    {
        return validate_xml10_text(ch.to_string());
    }
    let name = reference.decode().map_err(|error| error.to_string())?;
    let value = resolve_xml_entity(&name)
        .map(str::to_string)
        .ok_or_else(|| format!("unrecognized XML entity '&{name};'"))?;
    validate_xml10_text(value)
}

fn enforce_element_limits(element: &BytesStart<'_>, depth: usize) -> Result<()> {
    if depth >= MAX_XML_DEPTH {
        return Err(EngineError::Limit(format!(
            "XML nesting exceeds {MAX_XML_DEPTH} elements"
        )));
    }
    for (index, attribute) in element.attributes().enumerate() {
        attribute.map_err(|error| EngineError::Xml(format!("invalid XML attribute: {error}")))?;
        if index >= MAX_ATTRIBUTES_PER_ELEMENT {
            return Err(EngineError::Limit(format!(
                "XML element has more than {MAX_ATTRIBUTES_PER_ELEMENT} attributes"
            )));
        }
    }
    Ok(())
}

fn enforce_field_limit(value: &str) -> Result<()> {
    if value.len() > MAX_FIELD_BYTES {
        return Err(EngineError::Limit(format!(
            "XML field exceeds {MAX_FIELD_BYTES} bytes"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_xml10_text;

    #[test]
    fn xml10_rejects_bmp_noncharacters() {
        assert!(validate_xml10_text("\u{FFFE}".to_string()).is_err());
        assert!(validate_xml10_text("\u{FFFF}".to_string()).is_err());
        assert!(validate_xml10_text("\u{10000}".to_string()).is_ok());
    }
}
