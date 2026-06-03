//! UUID skills (local compute): generate v4 (random) or v7 (Unix-millisecond
//! timestamp + entropy), parse any UUID for version/variant, extract the
//! embedded timestamp from v1/v6/v7, and re-encode an existing UUID into
//! a compact URL-safe form.
//!
//! LLMs hallucinate UUIDv7 field layouts more often than not. This module
//! gives the model deterministic generators and a parser that reads the
//! actual bit layout per RFC 9562.
//!
//! ## Sources
//!
//! - RFC 9562 (UUID format / versions 1-8).
//! - RFC 4648 (base32, base64url alphabets).

use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// uuid_generate
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GenerateArgs {
    /// UUID version to generate: `v4` (random) or `v7` (timestamped). v7 is
    /// preferred for new identifiers in 2025+ because it's sortable by
    /// creation time.
    version: String,
    /// Number of UUIDs to generate. Defaults to 1; capped at 1000.
    #[serde(default)]
    count: Option<u32>,
    /// v7 only — RFC3339 timestamp to embed (default: now). Truncated to
    /// millisecond precision per RFC 9562.
    #[serde(default)]
    at: Option<String>,
}

pub struct UuidGenerate;
impl Skill for UuidGenerate {
    fn name(&self) -> &'static str {
        "uuid_generate"
    }
    fn description(&self) -> &'static str {
        "Generate one or more UUIDs. `version` is `v4` (random per RFC 9562 §5.4) or `v7` \
         (Unix-millisecond timestamp + random tail, sortable by creation time, RFC 9562 §5.7). \
         `count` defaults to 1, capped at 1000. For v7 you can pass `at` (RFC3339) to embed a \
         specific moment instead of `now` — millisecond precision per spec."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GenerateArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<GenerateArgs>()?;
            let count = args.count.unwrap_or(1).min(1000) as usize;
            let mut out = Vec::with_capacity(count);
            match args.version.trim().to_ascii_lowercase().as_str() {
                "v4" | "4" => {
                    for _ in 0..count {
                        out.push(Uuid::new_v4().to_string());
                    }
                }
                "v7" | "7" => {
                    let ts = match args.at.as_deref() {
                        None => uuid::Timestamp::now(uuid::NoContext),
                        Some(s) => {
                            let dt = DateTime::parse_from_rfc3339(s.trim())
                                .map_err(|e| {
                                    invalid(format!("could not parse `at` as RFC3339: {e}"))
                                })?
                                .with_timezone(&Utc);
                            uuid::Timestamp::from_unix(
                                uuid::NoContext,
                                dt.timestamp() as u64,
                                dt.timestamp_subsec_nanos(),
                            )
                        }
                    };
                    for _ in 0..count {
                        // v7 with a fixed timestamp produces identical UUIDs
                        // back-to-back; that's intentional for `at` use.
                        out.push(Uuid::new_v7(ts).to_string());
                    }
                }
                v => {
                    return Err(invalid(format!(
                        "unsupported UUID version `{v}` (try `v4` or `v7`)"
                    )));
                }
            }
            Ok(text_result(
                json!({
                    "version": args.version,
                    "count": out.len(),
                    "uuids": out,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Single v4 (random)",
                args: r#"{"version": "v4"}"#,
                note: Some("Returns one fully-random UUID per RFC 9562 §5.4."),
            },
            SkillExample {
                title: "Five v7 UUIDs at current time",
                args: r#"{"version": "v7", "count": 5}"#,
                note: Some("v7 is sortable by creation time — ideal for new IDs in 2025+."),
            },
            SkillExample {
                title: "v7 with a fixed embedded timestamp",
                args: r#"{"version": "v7", "at": "2024-06-01T12:34:56Z"}"#,
                note: Some("Embeds the requested Unix-millisecond timestamp; truncated to ms precision per spec."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Generate fresh identifiers for new records (prefer v7 for sortability).",
            "Reproduce a v7 UUID at a known historical moment for fixtures or tests.",
            "Bulk-create N IDs in one call.",
        ]
    }
}

// ---------------------------------------------------------------------------
// uuid_parse
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ParseArgs {
    /// UUID string in any canonical form (hyphenated, hex-only, urn:uuid:, braced).
    uuid: String,
}

pub struct UuidParse;
impl Skill for UuidParse {
    fn name(&self) -> &'static str {
        "uuid_parse"
    }
    fn description(&self) -> &'static str {
        "Parse a UUID and report its version (1-8 per RFC 9562) + variant (RFC, Microsoft, NCS, \
         future). For time-based versions (1, 6, 7) the embedded timestamp is decoded — v7 is \
         Unix-ms (RFC 9562 §5.7), v1/v6 are 100-ns intervals since 1582-10-15. Accepts \
         hyphenated, hex-only, urn:uuid:, and braced forms."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ParseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ParseArgs>()?;
            let parsed = Uuid::parse_str(args.uuid.trim())
                .map_err(|e| invalid(format!("could not parse `{}`: {e}", args.uuid)))?;
            let version = parsed.get_version_num();
            let variant = match parsed.get_variant() {
                uuid::Variant::RFC4122 => "RFC 4122/9562",
                uuid::Variant::Microsoft => "Microsoft",
                uuid::Variant::NCS => "NCS (legacy)",
                _ => "Future / reserved",
            };
            let mut obj = json!({
                "uuid": parsed.to_string(),
                "version": version,
                "variant": variant,
                "hyphenated": parsed.hyphenated().to_string(),
                "simple_hex": parsed.simple().to_string(),
                "urn": parsed.urn().to_string(),
            });
            if let Some(ts) = decode_timestamp(&parsed, version) {
                obj["embedded_timestamp_utc"] = json!(ts.to_rfc3339());
                obj["embedded_timestamp_unix_ms"] = json!(ts.timestamp_millis());
            }
            Ok(text_result(obj.to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Parse a v4 UUID",
                args: r#"{"uuid": "550e8400-e29b-41d4-a716-446655440000"}"#,
                note: Some("Reports version=4 (random), variant RFC 4122. No embedded timestamp."),
            },
            SkillExample {
                title: "Parse a v7 UUID and extract its timestamp",
                args: r#"{"uuid": "017fc3a4-58dc-7c00-8000-000000000001"}"#,
                note: Some("Reports version=7 and decodes the embedded Unix-ms timestamp."),
            },
            SkillExample {
                title: "Hex-only form (no hyphens)",
                args: r#"{"uuid": "550e8400e29b41d4a716446655440000"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Decode the creation-time embedded in a v7 ID without trusting an LLM's bit-layout guess.",
            "Confirm the UUID variant (RFC vs Microsoft GUID byte order) before processing.",
            "Normalize an incoming UUID to a chosen canonical form.",
        ]
    }
}

/// Decode the embedded timestamp for v1, v6, or v7 UUIDs. Returns None for
/// the other versions (which don't carry a timestamp).
fn decode_timestamp(u: &Uuid, version: usize) -> Option<DateTime<Utc>> {
    match version {
        1 | 6 => {
            // v1 timestamp: 100-ns intervals since 1582-10-15 UTC.
            // The `uuid` crate exposes get_timestamp() for v1/v6/v7.
            let ts = u.get_timestamp()?;
            let (secs, subsec_nanos) = ts.to_unix();
            Utc.timestamp_opt(secs as i64, subsec_nanos).single()
        }
        7 => {
            let ts = u.get_timestamp()?;
            let (secs, subsec_nanos) = ts.to_unix();
            Utc.timestamp_opt(secs as i64, subsec_nanos).single()
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// uuid_to_short
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ShortArgs {
    /// UUID to re-encode.
    uuid: String,
    /// Target encoding: `base32` (RFC 4648 §6, 26 chars), `base58` (Bitcoin
    /// alphabet, ~22 chars, no padding), or `base64url` (RFC 4648 §5, 22
    /// chars, URL-safe alphabet, no padding).
    encoding: String,
}

pub struct UuidToShort;
impl Skill for UuidToShort {
    fn name(&self) -> &'static str {
        "uuid_to_short"
    }
    fn description(&self) -> &'static str {
        "Re-encode an existing UUID into a compact, URL-safe representation: `base32` (26 chars), \
         `base58` (Bitcoin alphabet, ~22 chars), or `base64url` (22 chars, no padding). Useful \
         for short URLs, file names, or anywhere the 36-char canonical form is too long."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ShortArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ShortArgs>()?;
            let parsed = Uuid::parse_str(args.uuid.trim())
                .map_err(|e| invalid(format!("could not parse `{}`: {e}", args.uuid)))?;
            let bytes = parsed.as_bytes();
            let enc = args.encoding.trim().to_ascii_lowercase();
            let short = match enc.as_str() {
                "base32" => base32_encode(bytes),
                "base58" => base58_encode(bytes),
                "base64url" | "base64" => base64url_encode(bytes),
                e => {
                    return Err(invalid(format!(
                        "unsupported encoding `{e}` (try `base32`, `base58`, or `base64url`)"
                    )));
                }
            };
            Ok(text_result(
                json!({
                    "uuid": parsed.to_string(),
                    "encoding": enc,
                    "short": short,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Base64url (22 chars, no padding)",
                args: r#"{"uuid": "550e8400-e29b-41d4-a716-446655440000", "encoding": "base64url"}"#,
                note: Some("URL-safe alphabet (`-` and `_`), padding stripped."),
            },
            SkillExample {
                title: "Base32 (26 chars)",
                args: r#"{"uuid": "550e8400-e29b-41d4-a716-446655440000", "encoding": "base32"}"#,
                note: Some("RFC 4648 §6 alphabet, padding stripped."),
            },
            SkillExample {
                title: "Base58 (Bitcoin alphabet)",
                args: r#"{"uuid": "550e8400-e29b-41d4-a716-446655440000", "encoding": "base58"}"#,
                note: Some("~22 chars, no padding, no visually-confusing characters (0/O/I/l)."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Shorten a UUID for use in a URL slug or file name.",
            "Pick the encoding that survives copy-paste cleanest (base58 avoids 0/O/I/l).",
            "Compare encoding lengths before standardizing on one.",
        ]
    }
}

fn base64url_encode(bytes: &[u8]) -> String {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    URL_SAFE_NO_PAD.encode(bytes)
}

fn base32_encode(bytes: &[u8]) -> String {
    // RFC 4648 §6 alphabet, no padding.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity(26);
    let mut buf: u64 = 0;
    let mut bits: u32 = 0;
    for &b in bytes {
        buf = (buf << 8) | (b as u64);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buf >> bits) & 0x1f) as usize;
            out.push(ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buf << (5 - bits)) & 0x1f) as usize;
        out.push(ALPHABET[idx] as char);
    }
    out
}

fn base58_encode(bytes: &[u8]) -> String {
    // Bitcoin base58 alphabet.
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    // Convert big-endian bytes to a big integer, then to base58.
    let mut digits: Vec<u8> = Vec::with_capacity(22);
    let mut acc: Vec<u8> = bytes.to_vec();
    // Count leading zero bytes — each maps to a leading '1'.
    let leading_zeros = acc.iter().take_while(|&&b| b == 0).count();
    while acc.iter().any(|&b| b != 0) {
        let mut rem: u32 = 0;
        for byte in &mut acc {
            let v = (rem << 8) | (*byte as u32);
            *byte = (v / 58) as u8;
            rem = v % 58;
        }
        digits.push(rem as u8);
    }
    let mut out = String::with_capacity(22 + leading_zeros);
    for _ in 0..leading_zeros {
        out.push(ALPHABET[0] as char);
    }
    for d in digits.iter().rev() {
        out.push(ALPHABET[*d as usize] as char);
    }
    out
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "uuid"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "UUID generation (v4 / v7), parsing with version + variant + embedded-timestamp \
         extraction, and compact short-form encoding (base32 / base58 / base64url). Pure local \
         compute. RFC 9562 throughout."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `uuid_generate { version: \"v7\" }` — get a fresh ID, sortable by time.\n\
             2. `uuid_parse { uuid: \"<the uuid>\" }` — confirm version + decode the embedded timestamp.\n\
             3. `uuid_to_short { uuid: \"<the uuid>\", encoding: \"base64url\" }` — compact form for a URL slug.",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(UuidGenerate),
        Box::new(UuidParse),
        Box::new(UuidToShort),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v4_is_random_and_valid() {
        let u = Uuid::new_v4();
        assert_eq!(u.get_version_num(), 4);
    }

    #[test]
    fn v7_has_version_7() {
        let u = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
        assert_eq!(u.get_version_num(), 7);
    }

    #[test]
    fn parse_accepts_hyphenated_and_hex() {
        let h = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let s = Uuid::parse_str("550e8400e29b41d4a716446655440000").unwrap();
        assert_eq!(h, s);
    }

    #[test]
    fn base64url_is_22_chars_no_padding() {
        let u = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let s = base64url_encode(u.as_bytes());
        assert_eq!(s.len(), 22, "got: {s}");
        assert!(!s.contains('='), "padding leaked: {s}");
    }

    #[test]
    fn base32_is_26_chars_no_padding() {
        let u = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let s = base32_encode(u.as_bytes());
        assert_eq!(s.len(), 26, "got: {s}");
    }

    #[test]
    fn v7_timestamp_roundtrip() {
        let dt = DateTime::parse_from_rfc3339("2024-06-01T12:34:56Z")
            .unwrap()
            .with_timezone(&Utc);
        let ts = uuid::Timestamp::from_unix(
            uuid::NoContext,
            dt.timestamp() as u64,
            dt.timestamp_subsec_nanos(),
        );
        let u = Uuid::new_v7(ts);
        let back = decode_timestamp(&u, 7).unwrap();
        // v7 truncates to ms precision per RFC 9562 §5.7.
        assert!(
            (back.timestamp_millis() - dt.timestamp_millis()).abs() <= 1,
            "lost too much precision: {back} vs {dt}",
        );
    }
}
