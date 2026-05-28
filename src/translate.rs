//! Google Translate (keyless) via the public `translate_a/single` endpoint that
//! backs the Translate web widget. No API key, no account — a plain GET that
//! returns the translation plus the detected source language.
//!
//! Golden rules: keyless and plain-HTTP by default. This isn't a `SearchProvider`
//! (it transforms text rather than returning a ranked list), so it's exposed as
//! the standalone `translate` / `detect_language` tools rather than via the
//! registry, much like the `datetime` family.

use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::Value;

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";

/// The outcome of a translation request.
pub struct Translation {
    /// The translated text (all segments concatenated).
    pub text: String,
    /// The source language code — the one Google detected when `from` was
    /// "auto", or the requested source otherwise. Empty if not reported.
    pub source_lang: String,
}

/// Translate `text` into `to` (an ISO-639 code like "es", "fr", "ja", "zh-CN").
/// `from` is the source code, or "auto"/empty to auto-detect.
pub async fn translate(http: &Client, text: &str, to: &str, from: &str) -> Result<Translation> {
    let from = match from.trim() {
        "" => "auto",
        f => f,
    };
    let v: Value = http
        .get(ENDPOINT)
        .query(&[
            ("client", "gtx"),
            ("sl", from),
            ("tl", to.trim()),
            ("dt", "t"),
            ("q", text),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    parse(&v).ok_or_else(|| anyhow!("unexpected Google Translate response shape"))
}

/// Pull the translated text and detected source language out of the nested array
/// the endpoint returns: `[[["translated","source",…], …], …, "detected_lang", …]`.
fn parse(v: &Value) -> Option<Translation> {
    let segments = v.get(0)?.as_array()?;
    let mut text = String::new();
    for seg in segments {
        if let Some(s) = seg.get(0).and_then(|x| x.as_str()) {
            text.push_str(s);
        }
    }
    let source_lang = v
        .get(2)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string();
    if text.is_empty() {
        return None;
    }
    Some(Translation { text, source_lang })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_segments_and_detected_language() {
        // Shape of a real `dt=t` response: segment pairs, then nulls, then the
        // detected source language code.
        let v = serde_json::json!([
            [
                ["Hola, ", "Hello, ", null, null, 10],
                ["mundo", "world", null, null, 3]
            ],
            null,
            "en"
        ]);
        let t = parse(&v).unwrap();
        assert_eq!(t.text, "Hola, mundo");
        assert_eq!(t.source_lang, "en");
    }

    #[test]
    fn missing_translation_is_none() {
        assert!(parse(&serde_json::json!([null, null, "en"])).is_none());
        assert!(parse(&serde_json::json!({})).is_none());
    }
}
