//! Writing a corrected [`Feed`] back out in its original format.
//!
//! Preservation is the contract for inputs admitted by the parser's rewrite
//! safety gate: supported channel metadata and scalar product values survive
//! XML rewriting, the complete column union and source headers survive CSV,
//! and text is escaped. Unsupported structures are refused before this module.

use crate::error::{EngineError, Result};
use crate::model::{Feed, Format};
use std::io::Write;

pub const MAX_OUTPUT_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_OUTPUT_CELLS: usize = 16_000_000;

#[derive(Default)]
struct BoundedOutput(Vec<u8>);

impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > MAX_OUTPUT_BYTES {
            return Err(std::io::Error::other(
                "corrected CSV exceeds the 128 MiB output limit",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Serialize a feed to bytes in its own format.
pub fn serialize(feed: &Feed) -> Result<Vec<u8>> {
    ensure_rewrite_safe(feed)?;
    match feed.format {
        Format::Xml => Ok(to_xml(feed).into_bytes()),
        Format::Csv => to_csv(feed),
    }
}

pub(crate) fn ensure_rewrite_safe(feed: &Feed) -> Result<()> {
    if feed.rewrite_safe() {
        return Ok(());
    }
    let reason = if feed.rewrite_blockers.is_empty() {
        "the parsed feed is incomplete".to_string()
    } else {
        feed.rewrite_blockers.join("; ")
    };
    Err(EngineError::UnsafeRewrite(reason))
}

/// RSS 2.0 with the Google base namespace. The RSS-standard `title`, `link`, and
/// `description` are emitted unprefixed; every other attribute is emitted as
/// `g:<attr>`. Multivalue attributes are split back into repeated elements.
fn to_xml(feed: &Feed) -> String {
    const MULTIVALUE: &[&str] = &["additional_image_link", "product_type"];
    const UNPREFIXED: &[&str] = &["title", "link", "description"];

    let mut s = String::new();
    s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    s.push_str("<rss xmlns:g=\"http://base.google.com/ns/1.0\" version=\"2.0\">\n");
    s.push_str("  <channel>\n");

    if let Some(title) = &feed.channel.title {
        s.push_str(&format!("    <title>{}</title>\n", xml_escape(title)));
    }
    if let Some(link) = &feed.channel.link {
        s.push_str(&format!("    <link>{}</link>\n", xml_escape(link)));
    }
    if let Some(desc) = &feed.channel.description {
        s.push_str(&format!(
            "    <description>{}</description>\n",
            xml_escape(desc)
        ));
    }

    for p in &feed.products {
        s.push_str("    <item>\n");
        for (attr, value) in p.iter() {
            let emit = |s: &mut String, attr: &str, val: &str| {
                if UNPREFIXED.contains(&attr) {
                    s.push_str(&format!("      <{attr}>{}</{attr}>\n", xml_escape(val)));
                } else {
                    s.push_str(&format!("      <g:{attr}>{}</g:{attr}>\n", xml_escape(val)));
                }
            };
            if MULTIVALUE.contains(&attr) {
                for part in value.split(',') {
                    emit(&mut s, attr, part);
                }
            } else {
                emit(&mut s, attr, value);
            }
        }
        s.push_str("    </item>\n");
    }

    s.push_str("  </channel>\n");
    s.push_str("</rss>\n");
    s
}

/// CSV/TSV with the original delimiter and the full, stable column union.
fn to_csv(feed: &Feed) -> Result<Vec<u8>> {
    let columns = feed.column_union();
    if columns
        .len()
        .saturating_mul(feed.products.len().saturating_add(1))
        > MAX_OUTPUT_CELLS
    {
        return Err(EngineError::Limit(format!(
            "corrected CSV exceeds {MAX_OUTPUT_CELLS} rectangular cells"
        )));
    }
    let mut wtr = csv::WriterBuilder::new()
        .delimiter(feed.csv_delimiter)
        .from_writer(BoundedOutput::default());

    let header = columns
        .iter()
        .map(|column| {
            feed.csv_header_labels
                .get(column)
                .map_or(column.as_str(), String::as_str)
        })
        .collect::<Vec<_>>();
    wtr.write_record(&header)
        .map_err(|e| EngineError::Serialize(e.to_string()))?;

    for p in &feed.products {
        let row: Vec<&str> = columns.iter().map(|c| p.get_raw(c).unwrap_or("")).collect();
        wtr.write_record(&row)
            .map_err(|e| EngineError::Serialize(e.to_string()))?;
    }

    wtr.flush().map_err(EngineError::Io)?;
    wtr.into_inner()
        .map(|output| output.0)
        .map_err(|e| EngineError::Serialize(e.to_string()))
}

/// Escape the five XML metacharacters in text content/attribute values.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}
