//! `translate` / `detect_language` skills — Google Translate (keyless) via the
//! public `translate_a/single` endpoint that backs the Translate web widget. No
//! API key, no account — a plain GET that returns the translation plus the
//! detected source language.
//!
//! Golden rules: keyless and plain-HTTP by default. This isn't a `SearchProvider`
//! (it transforms text rather than returning a ranked list), so it's a pair of
//! standalone skills.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

const ENDPOINT: &str = "https://translate.googleapis.com/translate_a/single";

/// The outcome of a translation request.
pub struct Translation {
    /// The translated text (all segments concatenated).
    pub text: String,
    /// The source language code — detected when `from` was "auto", else the
    /// requested source. Empty if not reported.
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TranslateArgs {
    /// The text to translate.
    text: String,
    /// Target language as an ISO-639 code (e.g. "es", "fr", "de", "ja", "zh-CN").
    to: String,
    /// Source language code, or "auto" to detect it (default).
    #[serde(default)]
    from: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DetectLanguageArgs {
    /// The text whose language to detect.
    text: String,
}

pub struct Translate;

impl Skill for Translate {
    fn name(&self) -> &'static str {
        "translate"
    }
    fn description(&self) -> &'static str {
        "Translate text into another language with Google Translate (keyless, no API key). `to` is \
        an ISO-639 target code (es, fr, de, ja, zh-CN, …); `from` defaults to auto-detect. Returns \
        the translation and the detected source language."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TranslateArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<TranslateArgs>()?;
            let to = args.to.trim();
            if to.is_empty() {
                return Err(invalid("`to` (target language code) is required"));
            }
            let from = args.from.as_deref().map(str::trim).unwrap_or("auto");
            let key = format!("translate|{from}|{to}|{}", args.text);
            if let Some(cached) = server.retrieval_get(&key) {
                return Ok(text_result(cached));
            }
            let t = translate(&server.http, &args.text, to, from)
                .await
                .map_err(internal)?;
            let detected = if t.source_lang.is_empty() {
                from.to_string()
            } else {
                t.source_lang
            };
            let out = format!("Translation ({detected} → {to}):\n{}", t.text);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct DetectLanguage;

impl Skill for DetectLanguage {
    fn name(&self) -> &'static str {
        "detect_language"
    }
    fn description(&self) -> &'static str {
        "Detect the language of a piece of text using Google Translate (keyless). Returns the \
        detected ISO-639 language code."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DetectLanguageArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DetectLanguageArgs>()?;
            let key = format!("detect|{}", args.text);
            if let Some(cached) = server.retrieval_get(&key) {
                return Ok(text_result(cached));
            }
            let t = translate(&server.http, &args.text, "en", "auto")
                .await
                .map_err(internal)?;
            if t.source_lang.is_empty() {
                return Ok(text_result("Could not detect the language."));
            }
            let out = format!("Detected language: {}", t.source_lang);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(Translate), Box::new(DetectLanguage)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_segments_and_detected_language() {
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
