//! Small, dependency-free text helpers shared by the rules (which *detect*
//! problems) and the fixers (which *repair* them). Keeping them in one place
//! guarantees detection and repair agree — a rule never flags something the
//! fixer would handle differently.

/// Does this string appear to contain HTML markup? Looks for a `<` immediately
/// followed by a letter or `/` (an opening or closing tag), which avoids false
/// positives on bare `<` used as a less-than sign with a space after it.
pub fn has_html(s: &str) -> bool {
    let bytes = s.as_bytes();
    let Some(last_close) = bytes.iter().rposition(|&byte| byte == b'>') else {
        return false;
    };
    for i in 0..last_close {
        if bytes[i] == b'<' {
            if let Some(&next) = bytes.get(i + 1) {
                if next == b'/' || next.is_ascii_alphabetic() {
                    // Require a closing '>' somewhere after to look tag-like.
                    return true;
                }
            }
        }
    }
    false
}

/// Strip HTML tags and decode the handful of entities that actually show up in
/// product feeds, then collapse the resulting whitespace. Conservative: it
/// removes tags but never tries to interpret them (no list reconstruction etc.).
pub fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => {
                in_tag = true;
                // Substitute a space for the tag so removing a block-level tag
                // (</p>, <br>) never fuses the words on either side.
                out.push(' ');
            }
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");
    collapse_ws(&decoded)
}

/// Collapse all runs of whitespace to single spaces and trim the ends.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Fraction of alphabetic characters that are uppercase (0.0 if no letters).
pub fn uppercase_ratio(s: &str) -> f64 {
    let mut upper = 0usize;
    let mut letters = 0usize;
    for c in s.chars() {
        if c.is_alphabetic() {
            letters += 1;
            if c.is_uppercase() {
                upper += 1;
            }
        }
    }
    if letters == 0 {
        0.0
    } else {
        upper as f64 / letters as f64
    }
}

/// Heuristic: is this string "shouting" (mostly capitals)? Requires a minimum of
/// real letters so short tokens like "USB" or "XL" don't trip it.
pub fn is_shouty(s: &str) -> bool {
    let letters = s.chars().filter(|c| c.is_alphabetic()).count();
    letters >= 8 && s.chars().count() > 10 && uppercase_ratio(s) > 0.7
}

/// Words kept lowercase inside a title (unless they are the first word).
const SMALL_WORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "but", "by", "for", "if", "in", "nor", "of", "on", "or", "the",
    "to", "vs", "via", "with",
];

/// Convert a (typically ALL-CAPS) title to readable title case. Because the
/// input has already lost its original casing, this is treated as a safe repair:
/// we cannot recover "iPhone" from "IPHONE", but "Iphone" still beats "IPHONE".
/// Tokens that are purely non-alphabetic (sizes like `32"`, model numbers) and
/// short measurement units are passed through untouched.
pub fn smart_title_case(s: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let words: Vec<&str> = s.split_whitespace().collect();
    let last = words.len().saturating_sub(1);
    for (i, raw) in words.iter().enumerate() {
        let lower = raw.to_lowercase();
        // Preserve tokens with digits (model numbers, sizes) verbatim.
        if raw.chars().any(|c| c.is_ascii_digit()) {
            out.push((*raw).to_string());
            continue;
        }
        if i != 0 && i != last && SMALL_WORDS.contains(&lower.as_str()) {
            out.push(lower);
        } else {
            out.push(capitalize_first(&lower));
        }
    }
    out.join(" ")
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_inserts_space_for_tags() {
        assert_eq!(strip_html("<p>a</p><b>b</b>"), "a b");
        assert_eq!(strip_html("Keep cold!</p><br><b>Best"), "Keep cold! Best");
        assert_eq!(strip_html("no markup here"), "no markup here");
        assert_eq!(strip_html("x &amp; y"), "x & y");
    }

    #[test]
    fn has_html_detects_tags_not_bare_lt() {
        assert!(has_html("<b>hi</b>"));
        assert!(has_html("a<br>b"));
        assert!(!has_html("3 < 5 and 5 > 3"));
        assert!(!has_html("plain text"));
    }

    #[test]
    fn shouty_detection_has_a_floor() {
        assert!(is_shouty("PREMIUM STEEL BOTTLE"));
        assert!(!is_shouty("Premium Steel Bottle"));
        assert!(!is_shouty("USB")); // too few letters
        assert!(!is_shouty("XL")); // too short
    }

    #[test]
    fn title_case_keeps_digits_and_small_words() {
        assert_eq!(smart_title_case("PREMIUM STEEL 32OZ"), "Premium Steel 32OZ");
        assert_eq!(
            smart_title_case("THE BEST OF THE WORLD"),
            "The Best of the World"
        );
        // first and last word always capitalized even if "small"
        assert_eq!(smart_title_case("OF MICE AND MEN OF"), "Of Mice and Men Of");
    }

    #[test]
    fn collapse_ws_normalizes_runs() {
        assert_eq!(collapse_ws("  a   b\t c \n"), "a b c");
    }
}
