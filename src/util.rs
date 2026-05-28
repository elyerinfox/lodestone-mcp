//! Small shared text/HTML helpers used across providers and retrieval.

/// Convert an HTML fragment or document to readable plain text.
pub fn html_to_text(html: &str) -> String {
    collapse_blank_lines(&html2text::from_read(html.as_bytes(), 100))
}

/// Collapse all runs of whitespace into single spaces and trim.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Collapse 3+ consecutive blank lines down to a single blank line.
fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blanks = 0;
    for line in s.lines() {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim().to_string()
}

/// Truncate to `max` characters, appending a marker when truncation happens.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max).collect();
    format!("{truncated}\n\n[... truncated to {max} characters ...]")
}

/// Minimal HTML entity decoding for short strings (e.g. API-returned titles).
pub fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#215;", "×")
}
