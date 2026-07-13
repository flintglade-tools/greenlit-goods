//! Delimited-text (CSV/TSV) parser.
//!
//! Real merchant CSVs are exported by every tool under the sun, so we sniff the
//! delimiter (comma / tab / semicolon / pipe) instead of assuming one, run the
//! underlying reader in flexible mode so a ragged row becomes a *finding* rather
//! than a hard stop, and preserve every column — including ones we don't
//! recognize and even surplus fields past the header — so output round-trips.

use crate::error::{EngineError, Result};
use crate::finding::{Finding, Severity};
use crate::model::{canonical_attr, Feed, Format, Product};
use crate::parse::decode_lenient;
use crate::parse::{MAX_FIELD_BYTES, MAX_PRODUCTS};

pub fn parse_csv(bytes: &[u8]) -> Result<Feed> {
    let (text, enc_finding) = decode_lenient(bytes);

    let delimiter = sniff_delimiter(&text);
    let mut feed = Feed::new(Format::Csv);
    feed.csv_delimiter = delimiter;
    if let Some(f) = enc_finding {
        feed.block_rewrite("source text required a best-effort encoding recovery");
        feed.parse_findings.push(f);
    }

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .has_headers(true)
        .from_reader(text.as_bytes());

    // Canonicalize headers, disambiguating accidental duplicates so two distinct
    // source columns never silently merge.
    let source_headers: Vec<String> = match reader.headers() {
        Ok(h) => h.iter().map(str::to_string).collect(),
        Err(e) => return Err(EngineError::Csv(format!("could not read header row: {e}"))),
    };
    let headers = dedup_headers(
        source_headers
            .iter()
            .map(|value| canonical_attr(value))
            .collect(),
    );

    if headers.is_empty() {
        return Err(EngineError::Csv("header row was empty".into()));
    }
    feed.csv_columns = headers.clone();
    feed.csv_header_labels = headers.iter().cloned().zip(source_headers).collect();

    let mut record_no = 0u64;
    for result in reader.records() {
        record_no += 1;
        let record = match result {
            Ok(r) => r,
            Err(e) => {
                feed.mark_incomplete(format!("CSV row {record_no} was skipped"));
                feed.parse_findings.push(
                    Finding::new(
                        "GL-CSV-BADROW",
                        Severity::AtRisk,
                        format!("Row {record_no} could not be parsed and was skipped."),
                    )
                    .detail(format!("Reader error: {e}"))
                    .structural(),
                );
                continue;
            }
        };

        // Flag, but do not drop, rows whose field count disagrees with the header.
        if record.len() != headers.len() {
            feed.parse_findings.push(
                Finding::new(
                    "GL-CSV-RAGGED",
                    Severity::AtRisk,
                    format!(
                        "Row {record_no} has {} field(s) but the header defines {}.",
                        record.len(),
                        headers.len()
                    ),
                )
                .detail(
                    "Often caused by an unescaped comma or stray quote in a product field. \
                     Extra fields are preserved; missing fields are treated as blank.",
                )
                .structural(),
            );
        }

        let mut p = Product::new();
        p.source_record = Some(record_no);
        for (i, value) in record.iter().enumerate() {
            if value.len() > MAX_FIELD_BYTES {
                return Err(EngineError::Limit(format!(
                    "CSV row {record_no} field {} is {} bytes; maximum is {MAX_FIELD_BYTES}",
                    i + 1,
                    value.len()
                )));
            }
            let key = headers
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("extra_field_{}", i + 1));
            p.set(key, value.to_string());
        }
        if !p.is_empty() {
            if feed.products.len() >= MAX_PRODUCTS {
                return Err(EngineError::Limit(format!(
                    "product count exceeds {MAX_PRODUCTS}"
                )));
            }
            feed.products.push(p);
        }
    }

    if feed.products.is_empty() {
        return Err(EngineError::EmptyFeed);
    }
    Ok(feed)
}

/// Choose the delimiter by counting candidates outside quoted fields in the
/// first logical record. Defaults to comma when there is nothing to go on.
fn sniff_delimiter(text: &str) -> u8 {
    let candidates = [b',', b'\t', b';', b'|'];
    let mut counts = [0usize; 4];
    let bytes = text.as_bytes();
    let mut quoted = false;
    let mut index = 0usize;

    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'"' {
            if quoted && bytes.get(index + 1) == Some(&b'"') {
                index += 2;
                continue;
            }
            quoted = !quoted;
        } else if !quoted && (byte == b'\n' || byte == b'\r') {
            break;
        } else if !quoted {
            if let Some(position) = candidates.iter().position(|candidate| *candidate == byte) {
                counts[position] += 1;
            }
        }
        index += 1;
    }

    let mut best = b',';
    let mut best_count = 0usize;
    for (position, count) in counts.into_iter().enumerate() {
        if count > best_count {
            best = candidates[position];
            best_count = count;
        }
    }
    best
}

/// Ensure header names are unique by suffixing collisions (`color`, `color_2`).
fn dedup_headers(headers: Vec<String>) -> Vec<String> {
    use std::collections::HashMap;
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::with_capacity(headers.len());
    for h in headers {
        let h = if h.is_empty() {
            "column".to_string()
        } else {
            h
        };
        let count = seen.entry(h.clone()).or_insert(0);
        *count += 1;
        if *count == 1 {
            out.push(h);
        } else {
            out.push(format!("{h}_{}", *count));
        }
    }
    out
}
