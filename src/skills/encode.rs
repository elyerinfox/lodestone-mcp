//! Generic encoder / decoder skills (local compute): base64 / hex / URL /
//! HTML / ROT13 / Morse. Pure-Rust. LLMs hallucinate base64 strings almost
//! every time they're asked to compute one — deterministic encoders are the
//! right answer.

use std::sync::Arc;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TextArg {
    /// Text or bytes (UTF-8 string) to encode.
    text: String,
    /// `standard` (default, with padding) or `url_safe` (no padding, URL-safe alphabet).
    #[serde(default)]
    variant: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EncodedArg {
    /// Encoded text to decode.
    encoded: String,
    /// Variant when applicable (`standard` or `url_safe` for base64).
    #[serde(default)]
    variant: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PlainArg {
    /// Text to encode / decode.
    text: String,
}

// ---------- base64 ----------
pub struct EncodeBase64;
impl Skill for EncodeBase64 {
    fn name(&self) -> &'static str {
        "encode_base64"
    }
    fn description(&self) -> &'static str {
        "Base64-encode UTF-8 text. `variant` is `standard` (default) or `url_safe` (no padding). RFC 4648."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TextArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<TextArg>()?;
            let out = match a.variant.as_deref().unwrap_or("standard") {
                "url_safe" | "url" => URL_SAFE_NO_PAD.encode(a.text.as_bytes()),
                _ => STANDARD.encode(a.text.as_bytes()),
            };
            Ok(text_result(json!({"encoded": out}).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Standard",
                args: r#"{"text": "Hello, world!"}"#,
                note: Some("`SGVsbG8sIHdvcmxkIQ==` per RFC 4648."),
            },
            SkillExample {
                title: "URL-safe no padding",
                args: r#"{"text": "Hello?>", "variant": "url_safe"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Generate a deterministic base64 of a known string.",
            "Pick between standard + url_safe variants without guessing the alphabet.",
        ]
    }
}

pub struct DecodeBase64;
impl Skill for DecodeBase64 {
    fn name(&self) -> &'static str {
        "decode_base64"
    }
    fn description(&self) -> &'static str {
        "Base64-decode to UTF-8. `variant` = `standard` or `url_safe`. Errors if the result isn't valid UTF-8."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EncodedArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EncodedArg>()?;
            let bytes = match a.variant.as_deref().unwrap_or("standard") {
                "url_safe" | "url" => URL_SAFE_NO_PAD.decode(a.encoded.as_bytes()),
                _ => STANDARD.decode(a.encoded.as_bytes()),
            }
            .map_err(|e| invalid(format!("bad base64: {e}")))?;
            let s =
                String::from_utf8(bytes).map_err(|e| invalid(format!("not valid UTF-8: {e}")))?;
            Ok(text_result(json!({"decoded": s}).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Standard",
            args: r#"{"encoded": "SGVsbG8sIHdvcmxkIQ=="}"#,
            note: None,
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &["Decode base64 from headers / configs / tokens accurately."]
    }
}

// ---------- hex ----------
pub struct EncodeHex;
impl Skill for EncodeHex {
    fn name(&self) -> &'static str {
        "encode_hex"
    }
    fn description(&self) -> &'static str {
        "Hex-encode the UTF-8 bytes of `text`. Lowercase, no separators."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PlainArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PlainArg>()?;
            let out: String = a
                .text
                .as_bytes()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            Ok(text_result(json!({"encoded": out}).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Hex",
            args: r#"{"text": "abc"}"#,
            note: Some("Returns `616263`."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &["Compute hex of a known string without arithmetic mistakes."]
    }
}

pub struct DecodeHex;
impl Skill for DecodeHex {
    fn name(&self) -> &'static str {
        "decode_hex"
    }
    fn description(&self) -> &'static str {
        "Hex-decode to UTF-8. Whitespace and `0x` prefixes are stripped. Errors if the result isn't valid UTF-8."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EncodedArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EncodedArg>()?;
            let stripped: String = a
                .encoded
                .split(|c: char| c.is_ascii_whitespace() || c == ',')
                .map(|t| t.trim_start_matches("0x").trim_start_matches("0X"))
                .collect::<Vec<_>>()
                .join("");
            let cleaned: String = stripped.chars().filter(|c| c.is_ascii_hexdigit()).collect();
            if !cleaned.len().is_multiple_of(2) {
                return Err(invalid("odd number of hex digits"));
            }
            let mut bytes = Vec::with_capacity(cleaned.len() / 2);
            for i in (0..cleaned.len()).step_by(2) {
                bytes.push(
                    u8::from_str_radix(&cleaned[i..i + 2], 16)
                        .map_err(|e| invalid(format!("bad hex: {e}")))?,
                );
            }
            let s = String::from_utf8(bytes).map_err(|e| invalid(format!("not UTF-8: {e}")))?;
            Ok(text_result(json!({"decoded": s}).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Hex bytes",
            args: r#"{"encoded": "48 65 6c 6c 6f"}"#,
            note: Some("Returns `Hello`."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &["Decode hex pasted from a tool output."]
    }
}

// ---------- URL ----------
pub struct EncodeUrl;
impl Skill for EncodeUrl {
    fn name(&self) -> &'static str {
        "encode_url"
    }
    fn description(&self) -> &'static str {
        "Percent-encode for use in a URL path / query (RFC 3986). Alphanumerics + `-._~` pass through; everything else becomes `%XX`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PlainArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PlainArg>()?;
            let out: String = a
                .text
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                        (b as char).to_string()
                    } else {
                        format!("%{:02X}", b)
                    }
                })
                .collect();
            Ok(text_result(json!({"encoded": out}).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Query value",
                args: r#"{"text": "hello world!"}"#,
                note: Some("Returns `hello%20world%21`."),
            },
            SkillExample {
                title: "Unicode",
                args: r#"{"text": "café"}"#,
                note: Some("`café` -> `caf%C3%A9` (UTF-8 percent-encoded)."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Build a URL query value without missing the right reserved chars.",
            "Percent-encode user input before concatenating into a URL.",
        ]
    }
}

pub struct DecodeUrl;
impl Skill for DecodeUrl {
    fn name(&self) -> &'static str {
        "decode_url"
    }
    fn description(&self) -> &'static str {
        "Percent-decode a URL fragment to UTF-8. `+` is NOT translated to space (form-encoding); use only for path / query components per RFC 3986."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EncodedArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EncodedArg>()?;
            let chars: Vec<char> = a.encoded.chars().collect();
            let mut bytes = Vec::with_capacity(chars.len());
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '%' && i + 2 < chars.len() {
                    let hex: String = chars[i + 1..=i + 2].iter().collect();
                    if let Ok(b) = u8::from_str_radix(&hex, 16) {
                        bytes.push(b);
                        i += 3;
                        continue;
                    }
                }
                let mut buf = [0u8; 4];
                let s = chars[i].encode_utf8(&mut buf);
                bytes.extend_from_slice(s.as_bytes());
                i += 1;
            }
            let s = String::from_utf8(bytes)
                .map_err(|e| invalid(format!("decoded bytes aren't UTF-8: {e}")))?;
            Ok(text_result(json!({"decoded": s}).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Standard",
            args: r#"{"encoded": "hello%20world%21"}"#,
            note: Some("Returns `hello world!`."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Decode a query-string parameter for display.",
            "Round-trip a URL fragment after manipulation.",
        ]
    }
}

// ---------- HTML entities ----------
pub struct EncodeHtml;
impl Skill for EncodeHtml {
    fn name(&self) -> &'static str {
        "encode_html"
    }
    fn description(&self) -> &'static str {
        "Escape the five XML/HTML-special characters: `&` `<` `>` `\"` `'` → entity references."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PlainArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PlainArg>()?;
            let out: String = a
                .text
                .chars()
                .map(|c| match c {
                    '&' => "&amp;".to_string(),
                    '<' => "&lt;".to_string(),
                    '>' => "&gt;".to_string(),
                    '"' => "&quot;".to_string(),
                    '\'' => "&#39;".to_string(),
                    other => other.to_string(),
                })
                .collect();
            Ok(text_result(json!({"encoded": out}).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Escape <script>",
            args: r#"{"text": "<script>alert(1)</script>"}"#,
            note: Some("Safe for embedding as HTML text."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &["Make a user-supplied string safe for HTML embedding."]
    }
}

pub struct DecodeHtml;
impl Skill for DecodeHtml {
    fn name(&self) -> &'static str {
        "decode_html"
    }
    fn description(&self) -> &'static str {
        "Unescape common HTML entities (`&amp; &lt; &gt; &quot; &#NN; &#xHH;`). Does NOT cover the full HTML5 entity set — for that, use html_render upstream."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EncodedArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EncodedArg>()?;
            let mut out = String::with_capacity(a.encoded.len());
            let bytes: &[u8] = a.encoded.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == b'&' {
                    if let Some(end) = a.encoded[i..].find(';') {
                        let ent = &a.encoded[i + 1..i + end];
                        let replacement = match ent {
                            "amp" => Some('&'),
                            "lt" => Some('<'),
                            "gt" => Some('>'),
                            "quot" => Some('"'),
                            "apos" | "#39" => Some('\''),
                            "nbsp" => Some('\u{a0}'),
                            x if x.starts_with("#x") || x.starts_with("#X") => {
                                u32::from_str_radix(&x[2..], 16)
                                    .ok()
                                    .and_then(char::from_u32)
                            }
                            x if x.starts_with('#') => {
                                x[1..].parse::<u32>().ok().and_then(char::from_u32)
                            }
                            _ => None,
                        };
                        if let Some(c) = replacement {
                            out.push(c);
                            i += end + 1;
                            continue;
                        }
                    }
                }
                out.push(bytes[i] as char);
                i += 1;
            }
            Ok(text_result(json!({"decoded": out}).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Entity refs",
            args: r#"{"encoded": "5 &lt; 10 &amp; 3 &gt; 1"}"#,
            note: Some("Returns `5 < 10 & 3 > 1`."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &["Convert escaped HTML back to plain text."]
    }
}

// ---------- ROT13 ----------
pub struct EncodeRot13;
impl Skill for EncodeRot13 {
    fn name(&self) -> &'static str {
        "encode_rot13"
    }
    fn description(&self) -> &'static str {
        "ROT13 cipher — its own inverse. ASCII letters only, everything else passes through."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PlainArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PlainArg>()?;
            let out: String = a
                .text
                .chars()
                .map(|c| match c {
                    'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
                    'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
                    other => other,
                })
                .collect();
            Ok(text_result(json!({"encoded": out}).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Classic",
            args: r#"{"text": "Hello, world!"}"#,
            note: Some("Returns `Uryyb, jbeyq!`. Apply ROT13 again to decode."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "ROT13 a spoiler / puzzle clue accurately.",
            "Confirm a ROT13'd string round-trips correctly.",
        ]
    }
}

// ---------- Morse ----------
pub struct EncodeMorse;
impl Skill for EncodeMorse {
    fn name(&self) -> &'static str {
        "encode_morse"
    }
    fn description(&self) -> &'static str {
        "Convert ASCII text to International Morse code. Words separated by ` / `, letters by spaces. Unrecognized chars become `?`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PlainArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PlainArg>()?;
            let mut out = String::new();
            for (i, word) in a.text.split_whitespace().enumerate() {
                if i > 0 {
                    out.push_str(" / ");
                }
                let mut first = true;
                for c in word.to_ascii_uppercase().chars() {
                    if !first {
                        out.push(' ');
                    }
                    first = false;
                    out.push_str(morse_for(c));
                }
            }
            Ok(text_result(json!({"encoded": out}).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "SOS",
                args: r#"{"text": "SOS"}"#,
                note: Some("Returns `... --- ...`."),
            },
            SkillExample {
                title: "Hello world",
                args: r#"{"text": "HELLO WORLD"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &["Compute Morse for a known message without lookup errors."]
    }
}

fn morse_for(c: char) -> &'static str {
    match c {
        'A' => ".-",
        'B' => "-...",
        'C' => "-.-.",
        'D' => "-..",
        'E' => ".",
        'F' => "..-.",
        'G' => "--.",
        'H' => "....",
        'I' => "..",
        'J' => ".---",
        'K' => "-.-",
        'L' => ".-..",
        'M' => "--",
        'N' => "-.",
        'O' => "---",
        'P' => ".--.",
        'Q' => "--.-",
        'R' => ".-.",
        'S' => "...",
        'T' => "-",
        'U' => "..-",
        'V' => "...-",
        'W' => ".--",
        'X' => "-..-",
        'Y' => "-.--",
        'Z' => "--..",
        '0' => "-----",
        '1' => ".----",
        '2' => "..---",
        '3' => "...--",
        '4' => "....-",
        '5' => ".....",
        '6' => "-....",
        '7' => "--...",
        '8' => "---..",
        '9' => "----.",
        '.' => ".-.-.-",
        ',' => "--..--",
        '?' => "..--..",
        '\'' => ".----.",
        '!' => "-.-.--",
        '/' => "-..-.",
        '(' => "-.--.",
        ')' => "-.--.-",
        '&' => ".-...",
        ':' => "---...",
        ';' => "-.-.-.",
        '=' => "-...-",
        '+' => ".-.-.",
        '-' => "-....-",
        '_' => "..--.-",
        '"' => ".-..-.",
        '$' => "...-..-",
        '@' => ".--.-.",
        _ => "?",
    }
}

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "encode"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Generic encoders / decoders: base64 (standard + url_safe), hex, URL percent-encoding, HTML entities, ROT13, Morse. Pure local compute."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some("1. `encode_base64 { text: \"<your data>\" }` to encode.\n2. `decode_base64 { encoded: \"<base64>\" }` to round-trip.\n3. For URL building use `encode_url`; for HTML safety use `encode_html`.")
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(EncodeBase64),
        Box::new(DecodeBase64),
        Box::new(EncodeHex),
        Box::new(DecodeHex),
        Box::new(EncodeUrl),
        Box::new(DecodeUrl),
        Box::new(EncodeHtml),
        Box::new(DecodeHtml),
        Box::new(EncodeRot13),
        Box::new(EncodeMorse),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base64_roundtrip() {
        let s = STANDARD.encode("Hello, world!".as_bytes());
        assert_eq!(s, "SGVsbG8sIHdvcmxkIQ==");
        let b = STANDARD.decode(s.as_bytes()).unwrap();
        assert_eq!(b, b"Hello, world!");
    }
    #[test]
    fn rot13_is_self_inverse() {
        // Direct call of the rotation rule on each char.
        let s = "Hello";
        let r: String = s
            .chars()
            .map(|c| match c {
                'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
                'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
                o => o,
            })
            .collect();
        let rr: String = r
            .chars()
            .map(|c| match c {
                'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
                'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
                o => o,
            })
            .collect();
        assert_eq!(rr, s);
    }
    #[test]
    fn morse_sos() {
        let out: String = "SOS".chars().map(morse_for).collect::<Vec<_>>().join(" ");
        assert_eq!(out, "... --- ...");
    }
}
