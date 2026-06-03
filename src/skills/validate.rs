//! Validator skills (local compute): Luhn (credit card / EMV PAN), ISBN-10
//! and ISBN-13, IBAN, ISSN. Pure-Rust. LLMs frequently fabricate IDs that
//! look right but fail the checksum — deterministic validation catches
//! the bad ones before they propagate.
//!
//! ## Sources
//!
//! - ISO/IEC 7812-1 (PAN format + Luhn).
//! - ISO 2108 (ISBN-13) + the older ISBN-10 mod-11 algorithm.
//! - ISO 13616 (IBAN structure + mod-97 check).
//! - ISO 3297 (ISSN structure + checksum).

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InputArg {
    /// Input string (spaces / hyphens stripped before validation).
    input: String,
}

// ---------- Luhn ----------
pub struct ValidateLuhn;
impl Skill for ValidateLuhn {
    fn name(&self) -> &'static str {
        "validate_luhn"
    }
    fn description(&self) -> &'static str {
        "Luhn-checksum validation (RFC 7812-1) for credit card PANs, IMEIs, and a few other ID systems. Strips spaces / hyphens; rejects anything non-digit after stripping. Returns `valid` + the issuer guess for common card BINs."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<InputArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<InputArg>()?;
            let digits = strip(&a.input);
            if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
                return Err(invalid(
                    "input must contain digits (spaces / hyphens are stripped)",
                ));
            }
            let valid = luhn(&digits);
            let issuer = guess_issuer(&digits);
            Ok(text_result(
                json!({
                    "input": a.input,
                    "digits": digits.len(),
                    "valid": valid,
                    "issuer_guess": issuer,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Valid test PAN",
                args: r#"{"input": "4242 4242 4242 4242"}"#,
                note: Some("Standard Stripe test PAN — passes Luhn, issuer_guess=Visa."),
            },
            SkillExample {
                title: "Invalid",
                args: r#"{"input": "1234 5678 9012 3456"}"#,
                note: Some("Fails Luhn."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Validate a credit card / IMEI checksum before submitting it.",
            "Spot LLM-fabricated PANs (they almost always fail Luhn).",
        ]
    }
}

fn luhn(digits: &str) -> bool {
    let mut sum = 0u32;
    let bytes = digits.as_bytes();
    let n = bytes.len();
    for (i, b) in bytes.iter().enumerate() {
        let d = (b - b'0') as u32;
        let from_right = n - 1 - i;
        let dbl = (from_right % 2) == 1;
        sum += if dbl {
            let v = d * 2;
            if v > 9 {
                v - 9
            } else {
                v
            }
        } else {
            d
        };
    }
    sum.is_multiple_of(10)
}

fn guess_issuer(d: &str) -> Option<&'static str> {
    let prefix2: u32 = d.get(..2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let prefix4: u32 = d.get(..4).and_then(|s| s.parse().ok()).unwrap_or(0);
    let first = d.chars().next()?;
    Some(match (first, prefix2, prefix4) {
        ('4', _, _) => "Visa",
        (_, 51..=55, _) => "Mastercard",
        (_, _, 2221..=2720) => "Mastercard",
        (_, 34 | 37, _) => "American Express",
        (_, 60 | 62 | 64 | 65, _) => "Discover / RuPay / UnionPay",
        (_, 35, _) => "JCB",
        (_, 30 | 36 | 38 | 39, _) => "Diners Club",
        _ => return None,
    })
}

fn strip(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect()
}

// ---------- ISBN ----------
pub struct ValidateIsbn;
impl Skill for ValidateIsbn {
    fn name(&self) -> &'static str {
        "validate_isbn"
    }
    fn description(&self) -> &'static str {
        "Validate ISBN-10 (mod 11) or ISBN-13 (mod 10). Recognized by length after stripping spaces / hyphens. ISBN-10's check character can be `X`; ISBN-13 must be all digits."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<InputArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<InputArg>()?;
            let s = strip(&a.input);
            let (form, valid) = match s.len() {
                10 => ("ISBN-10", isbn10_valid(&s)),
                13 => ("ISBN-13", isbn13_valid(&s)),
                _ => {
                    return Err(invalid(format!(
                        "ISBN must be 10 or 13 digits after stripping, got {}",
                        s.len()
                    )))
                }
            };
            Ok(text_result(
                json!({"input": a.input, "form": form, "valid": valid}).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "ISBN-10 with X check",
                args: r#"{"input": "0-306-40615-2"}"#,
                note: Some("Classic mod-11 check digit example."),
            },
            SkillExample {
                title: "ISBN-13",
                args: r#"{"input": "978-3-16-148410-0"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Validate an ISBN before storing or looking it up.",
            "Catch typos in book IDs.",
        ]
    }
}

fn isbn10_valid(s: &str) -> bool {
    if s.len() != 10 {
        return false;
    }
    let mut sum = 0i32;
    for (i, c) in s.chars().enumerate() {
        let v = match c {
            '0'..='9' => (c as i32) - ('0' as i32),
            'X' | 'x' if i == 9 => 10,
            _ => return false,
        };
        sum += v * ((10 - i) as i32);
    }
    sum % 11 == 0
}

fn isbn13_valid(s: &str) -> bool {
    if s.len() != 13 {
        return false;
    }
    if !s.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let mut sum = 0;
    for (i, c) in s.chars().enumerate() {
        let v = (c as i32) - ('0' as i32);
        sum += if i % 2 == 0 { v } else { 3 * v };
    }
    sum % 10 == 0
}

// ---------- IBAN ----------
pub struct ValidateIban;
impl Skill for ValidateIban {
    fn name(&self) -> &'static str {
        "validate_iban"
    }
    fn description(&self) -> &'static str {
        "Validate an IBAN per ISO 13616: 2-letter country, 2-digit check, up to 30 alphanumerics; mod-97 check on the rearranged + numeric-translated string must equal 1. Spaces are stripped."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<InputArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<InputArg>()?;
            let s: String = a.input.chars().filter(|c| !c.is_whitespace()).collect();
            let upper = s.to_ascii_uppercase();
            if upper.len() < 5 || upper.len() > 34 {
                return Err(invalid("IBAN must be 5..=34 chars after stripping spaces"));
            }
            let valid = iban_mod97(&upper);
            let country = if upper.len() >= 2 {
                upper[..2].to_string()
            } else {
                String::new()
            };
            Ok(text_result(
                json!({"input": a.input, "country": country, "valid": valid}).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "German IBAN (Bundesbank test)",
                args: r#"{"input": "DE89 3704 0044 0532 0130 00"}"#,
                note: Some("Returns valid=true (canonical example)."),
            },
            SkillExample {
                title: "British IBAN",
                args: r#"{"input": "GB82 WEST 1234 5698 7654 32"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Validate an IBAN before sending payment instructions.",
            "Catch typos in bank account numbers.",
        ]
    }
}

fn iban_mod97(iban: &str) -> bool {
    // Move first 4 to the end.
    if iban.len() < 5 {
        return false;
    }
    let rearranged: String = format!("{}{}", &iban[4..], &iban[..4]);
    // Replace each letter A..Z with 10..35.
    let mut translated = String::new();
    for c in rearranged.chars() {
        match c {
            '0'..='9' => translated.push(c),
            'A'..='Z' => translated.push_str(&((c as u32 - 'A' as u32 + 10).to_string())),
            _ => return false,
        }
    }
    // Stream-mod-97.
    let mut rem: u64 = 0;
    for c in translated.chars() {
        let d = (c as u8 - b'0') as u64;
        rem = (rem * 10 + d) % 97;
    }
    rem == 1
}

// ---------- ISSN ----------
pub struct ValidateIssn;
impl Skill for ValidateIssn {
    fn name(&self) -> &'static str {
        "validate_issn"
    }
    fn description(&self) -> &'static str {
        "Validate an ISSN per ISO 3297 — 8 chars, last is mod-11 check digit (`X` for 10). Hyphen between groups is optional."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<InputArg>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<InputArg>()?;
            let s = strip(&a.input);
            if s.len() != 8 {
                return Err(invalid(format!(
                    "ISSN must be 8 chars after stripping, got {}",
                    s.len()
                )));
            }
            let mut sum = 0i32;
            for (i, c) in s.chars().enumerate() {
                let v = match c {
                    '0'..='9' => (c as i32) - ('0' as i32),
                    'X' | 'x' if i == 7 => 10,
                    _ => return Err(invalid("ISSN must be digits (or X for the check digit)")),
                };
                sum += v * ((8 - i) as i32);
            }
            Ok(text_result(
                json!({"input": a.input, "valid": sum % 11 == 0}).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Nature",
                args: r#"{"input": "0028-0836"}"#,
                note: Some("Returns valid=true."),
            },
            SkillExample {
                title: "With X check digit",
                args: r#"{"input": "0317-8471"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &["Validate a journal ISSN before relying on it for a citation."]
    }
}

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "validate"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Checksum validators for Luhn (credit cards / IMEI), ISBN-10 and ISBN-13, IBAN (ISO 13616), and ISSN (ISO 3297). Pure local compute. Catches LLM-fabricated identifiers that look right but fail the check."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some("1. `validate_luhn { input: \"<PAN>\" }` — credit-card / IMEI.\n2. `validate_isbn { input: \"<isbn>\" }` — book id.\n3. `validate_iban { input: \"<IBAN>\" }` — bank account.")
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(ValidateLuhn),
        Box::new(ValidateIsbn),
        Box::new(ValidateIban),
        Box::new(ValidateIssn),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn luhn_valid_visa_test() {
        assert!(luhn("4242424242424242"));
    }
    #[test]
    fn luhn_invalid() {
        assert!(!luhn("1234567890123456"));
    }
    #[test]
    fn isbn10_canonical() {
        assert!(isbn10_valid("0306406152"));
    }
    #[test]
    fn isbn13_example() {
        assert!(isbn13_valid("9783161484100"));
    }
    #[test]
    fn iban_german_test() {
        assert!(iban_mod97("DE89370400440532013000"));
    }
    #[test]
    fn iban_british_test() {
        assert!(iban_mod97("GB82WEST12345698765432"));
    }
}
