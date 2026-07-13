//! Parsing: turn raw feed bytes into a normalized [`Feed`].
//!
//! Two responsibilities live here that are shared by both concrete parsers:
//! detecting the format, and decoding bytes to text in a way that tolerates the
//! encoding messes real feeds ship with (UTF-8/UTF-16 BOMs, stray Windows-1252).

pub mod csv;
pub mod xml;

use crate::error::{EngineError, Result};
use crate::finding::{Finding, Severity};
use crate::model::{Feed, Format};

pub const MAX_INPUT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PRODUCTS: usize = 250_000;
pub const MAX_FIELD_BYTES: usize = 1_000_000;
pub const MAX_XML_DEPTH: usize = 64;
pub const MAX_ATTRIBUTES_PER_ELEMENT: usize = 256;

/// Auto-detect the feed format from a byte sample.
///
/// The heuristic is deliberately simple and robust: after skipping any BOM and
/// leading whitespace, a feed that begins with `<` is XML; anything else with
/// printable content is treated as delimited text. Returns `None` only for
/// effectively empty input.
pub fn detect_format(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let (decoded, _) = decode_lenient(bytes);
        let first = decoded.chars().find(|c| !c.is_whitespace())?;
        return Some(if first == '<' {
            Format::Xml
        } else {
            Format::Csv
        });
    }
    let body = strip_bom(bytes);
    let first = body.iter().copied().find(|b| !b.is_ascii_whitespace())?;
    if first == b'<' {
        Some(Format::Xml)
    } else {
        Some(Format::Csv)
    }
}

/// Parse bytes into a [`Feed`] using the given format.
pub fn parse(bytes: &[u8], format: Format) -> Result<Feed> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(EngineError::Limit(format!(
            "input is {} bytes; maximum is {MAX_INPUT_BYTES}",
            bytes.len()
        )));
    }
    match format {
        Format::Xml => xml::parse_xml(bytes),
        Format::Csv => csv::parse_csv(bytes),
    }
}

/// Parse bytes, auto-detecting the format first.
pub fn parse_auto(bytes: &[u8]) -> Result<Feed> {
    let format = detect_format(bytes).ok_or(EngineError::UnknownFormat)?;
    parse(bytes, format)
}

/// Strip a leading UTF-8 or UTF-16 byte-order mark, returning the remainder.
fn strip_bom(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        &bytes[2..]
    } else {
        bytes
    }
}

/// Decode arbitrary feed bytes to a `String`, leniently.
///
/// Strategy, in order: honor a UTF-8/UTF-16 BOM if present; otherwise try strict
/// UTF-8; and if that fails, fall back to Windows-1252 (a superset of Latin-1
/// that maps every byte to *some* character, so decoding never loses data). When
/// a fallback or lossy replacement happens, a `Finding` is returned so the
/// report can flag the encoding problem — this is a real cause of mojibake in
/// titles and descriptions that hurts listing quality.
pub(crate) fn decode_lenient(bytes: &[u8]) -> (String, Option<Finding>) {
    // UTF-16 BOMs — decode via encoding_rs which strips the BOM itself.
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let (cow, _, had_err) = encoding_rs::UTF_16LE.decode(&bytes[2..]);
        return (cow.into_owned(), encoding_finding("UTF-16LE", had_err));
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let (cow, _, had_err) = encoding_rs::UTF_16BE.decode(&bytes[2..]);
        return (cow.into_owned(), encoding_finding("UTF-16BE", had_err));
    }

    let body = strip_bom(bytes);
    match std::str::from_utf8(body) {
        Ok(s) => (s.to_string(), None),
        Err(_) => {
            let (cow, _, _) = encoding_rs::WINDOWS_1252.decode(body);
            (
                cow.into_owned(),
                Some(
                    Finding::new(
                        "GL-ENCODING",
                        Severity::AtRisk,
                        "Feed is not valid UTF-8; decoded as Windows-1252 as a best effort.",
                    )
                    .detail(
                        "Non-UTF-8 feeds frequently produce garbled accented characters \
                         (mojibake) in titles and descriptions. Re-export the feed as UTF-8.",
                    )
                    .structural(),
                ),
            )
        }
    }
}

fn encoding_finding(label: &str, had_err: bool) -> Option<Finding> {
    if had_err {
        Some(
            Finding::new(
                "GL-ENCODING",
                Severity::AtRisk,
                format!(
                    "Feed decoded as {label} but contained invalid sequences that were replaced."
                ),
            )
            .detail("Replacement characters may appear in product text. Re-export as UTF-8.")
            .structural(),
        )
    } else {
        None
    }
}
