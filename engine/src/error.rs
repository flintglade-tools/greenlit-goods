//! Fatal error taxonomy for the engine.
//!
//! A guiding principle: *recoverable* problems in a feed (a ragged CSV row, a
//! product missing a required attribute, an invalid GTIN) are **not** errors.
//! They are [`crate::finding::Finding`]s — that is the entire product. An
//! `EngineError` is reserved for situations where we genuinely cannot produce a
//! report at all: the bytes are not a feed in the claimed format, or the feed
//! is structurally empty.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    /// Format auto-detection failed and the caller did not specify one.
    #[error("could not auto-detect the feed format; pass --format xml or --format csv")]
    UnknownFormat,

    /// The XML was malformed beyond the point of recovery (not just a bad item).
    #[error("the XML feed could not be parsed: {0}")]
    Xml(String),

    /// The CSV had no usable header row, or was otherwise unreadable.
    #[error("the CSV/TSV feed could not be parsed: {0}")]
    Csv(String),

    /// Parsing succeeded but yielded zero products.
    #[error("the feed parsed successfully but contained no products")]
    EmptyFeed,

    /// Writing a corrected feed back out failed.
    #[error("failed to serialize the corrected feed: {0}")]
    Serialize(String),

    /// The parser could audit the feed, but cannot prove a rewrite preserves it.
    #[error("refusing to rewrite this feed safely: {0}")]
    UnsafeRewrite(String),

    #[error("invalid audit options: {0}")]
    InvalidOptions(String),

    #[error("feed exceeds a safety limit: {0}")]
    Limit(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, EngineError>;
