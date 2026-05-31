//! Small shared text/HTML helpers used across providers and retrieval.

/// Constant-time byte-slice equality — no early return on first mismatch, so a
/// matching prefix can't be discovered via response timing. Used for bearer
/// token checks.
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Convert an HTML fragment or document to readable plain text.
pub fn html_to_text(html: &str) -> String {
    collapse_blank_lines(&html2text::from_read(html.as_bytes(), 100))
}

/// Collapse all runs of whitespace into single spaces and trim.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Minimal percent-encoding for a URL path/query segment: alphanumerics and
/// `-`, `_`, `.`, `~` pass through; every other byte becomes `%XX`. RFC 3986
/// unreserved-character set. Avoids pulling in the `url` crate for what is
/// usually a one-line escape of a search query.
///
/// Replaces ~8 byte-identical `url_enc` / `url_encode` / `urlencoding`
/// helpers that used to live in `weather`, `peeringdb`, `eia`, `grid`,
/// `osm`, `huggingface`, `yahoo`, and `satellite`.
pub fn url_enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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

/// Compact human byte size (e.g. "36.3 MB").
pub fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Compact human count (e.g. "13.0B", "21.3K") for star/pull tallies.
pub fn human_count(n: i64) -> String {
    let a = n.unsigned_abs() as f64;
    let (v, suffix) = if a >= 1e9 {
        (a / 1e9, "B")
    } else if a >= 1e6 {
        (a / 1e6, "M")
    } else if a >= 1e3 {
        (a / 1e3, "K")
    } else {
        return n.to_string();
    };
    format!("{}{v:.1}{suffix}", if n < 0 { "-" } else { "" })
}

/// Prefix every line of `s` with `prefix` (for indenting multi-line snippets).
pub fn indent(s: &str, prefix: &str) -> String {
    s.lines()
        .map(|l| format!("{prefix}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
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
