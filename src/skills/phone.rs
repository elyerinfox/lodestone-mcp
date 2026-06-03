//! Phone number skills (local compute): parse + validate + format.
//! Pure-Rust via the `phonenumber` crate (the official port of Google
//! libphonenumber's logic). LLMs fabricate international dial codes
//! constantly; this gives the model deterministic E.164 normalization,
//! country detection, region-specific formatting, and a validity verdict.
//!
//! ## Sources
//!
//! - ITU-T E.164: international public telecommunication numbering plan.
//! - Google libphonenumber metadata (the same data the `phonenumber`
//!   Rust crate ships). Updated quarterly.

use std::str::FromStr;
use std::sync::Arc;

use futures::future::BoxFuture;
use phonenumber::country::Id as CountryId;
use phonenumber::{parse, Mode, PhoneNumber};
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// phone_parse
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ParseArgs {
    /// Phone number as entered by a human — international (`+44 20 ...`),
    /// national with default country, or with separators / parens / spaces.
    number: String,
    /// Optional ISO-3166-1 alpha-2 country code (e.g. `"US"`, `"GB"`, `"JP"`)
    /// for parsing national-format numbers without a `+` prefix. If omitted,
    /// the input must start with `+`.
    #[serde(default)]
    default_country: Option<String>,
}

pub struct PhoneParse;
impl Skill for PhoneParse {
    fn name(&self) -> &'static str {
        "phone_parse"
    }
    fn description(&self) -> &'static str {
        "Parse a phone number and report E.164 (`+CCNNNNNNNNNNN`), country code, region (ISO-3166 \
         alpha-2), number type (mobile, fixed_line, voip, …), validity, and possibility flags. \
         Accepts international (`+44 20 ...`) or national format with a `default_country` hint. \
         Local compute via the `phonenumber` crate."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ParseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ParseArgs>()?;
            let default_country = parse_country(args.default_country.as_deref())?;
            let parsed = parse(default_country, &args.number)
                .map_err(|e| invalid(format!("could not parse `{}`: {e}", args.number)))?;
            Ok(text_result(report(&parsed).to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "International format (UK)",
                args: r#"{"number": "+44 20 7946 0958"}"#,
                note: Some("Returns E.164 +442079460958, country=GB, fixed_line, valid."),
            },
            SkillExample {
                title: "National format with country hint",
                args: r#"{"number": "(415) 555-1234", "default_country": "US"}"#,
                note: Some("Returns E.164 +14155551234, country=US."),
            },
            SkillExample {
                title: "Japanese mobile",
                args: r#"{"number": "+81 90-1234-5678"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Normalize a user-entered phone number to E.164 before storing it.",
            "Pick the right country dialing code without hallucinating it.",
            "Decide whether a number is a mobile (can receive SMS) before sending one.",
        ]
    }
}

// ---------------------------------------------------------------------------
// phone_format
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FormatArgs {
    /// Phone number to format (international or national, see `phone_parse`).
    number: String,
    /// Format style: `e164` (`+14155551234`), `international` (`+1 415-555-1234`),
    /// `national` (`(415) 555-1234`), `rfc3966` (`tel:+1-415-555-1234`).
    /// Defaults to `e164`.
    #[serde(default)]
    style: Option<String>,
    /// Default country (ISO-3166 alpha-2) for parsing national input. Optional.
    #[serde(default)]
    default_country: Option<String>,
}

pub struct PhoneFormat;
impl Skill for PhoneFormat {
    fn name(&self) -> &'static str {
        "phone_format"
    }
    fn description(&self) -> &'static str {
        "Re-format a phone number into one of the standard display styles: `e164` \
         (`+14155551234`), `international` (`+1 415-555-1234`), `national` (`(415) 555-1234`), \
         or `rfc3966` (`tel:+1-415-555-1234`). Default style is `e164`. Local compute."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FormatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<FormatArgs>()?;
            let default_country = parse_country(args.default_country.as_deref())?;
            let parsed = parse(default_country, &args.number)
                .map_err(|e| invalid(format!("could not parse `{}`: {e}", args.number)))?;
            let mode = match args
                .style
                .as_deref()
                .unwrap_or("e164")
                .to_ascii_lowercase()
                .as_str()
            {
                "e164" | "" => Mode::E164,
                "international" | "intl" => Mode::International,
                "national" => Mode::National,
                "rfc3966" | "tel" => Mode::Rfc3966,
                s => {
                    return Err(invalid(format!(
                        "unknown style `{s}` (try e164/international/national/rfc3966)"
                    )))
                }
            };
            let formatted = parsed.format().mode(mode).to_string();
            Ok(text_result(
                json!({
                    "input": args.number,
                    "style": args.style.unwrap_or_else(|| "e164".into()),
                    "formatted": formatted,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "E.164 (default)",
                args: r#"{"number": "+1 (415) 555-1234"}"#,
                note: Some("Returns `+14155551234`."),
            },
            SkillExample {
                title: "International display",
                args: r#"{"number": "+44 20 7946 0958", "style": "international"}"#,
                note: Some("Returns `+44 20 7946 0958`."),
            },
            SkillExample {
                title: "tel: URI for a hyperlink",
                args: r#"{"number": "+14155551234", "style": "rfc3966"}"#,
                note: Some("Returns `tel:+1-415-555-1234`."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Re-format a stored E.164 number for display in a UI.",
            "Generate a `tel:` URI for a click-to-call hyperlink.",
            "Compare two numbers by normalizing both to E.164 first.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn parse_country(s: Option<&str>) -> Result<Option<CountryId>, McpError> {
    let raw = match s.map(str::trim).filter(|x| !x.is_empty()) {
        Some(x) => x,
        None => return Ok(None),
    };
    CountryId::from_str(&raw.to_ascii_uppercase())
        .map(Some)
        .map_err(|_| invalid(format!("unknown ISO-3166 alpha-2 country code `{raw}`")))
}

fn report(p: &PhoneNumber) -> serde_json::Value {
    let valid = phonenumber::is_valid(p);
    let country: String = p
        .country()
        .id()
        .map(|c| format!("{c:?}"))
        .unwrap_or_else(|| "unknown".to_string());
    json!({
        "e164": p.format().mode(Mode::E164).to_string(),
        "international": p.format().mode(Mode::International).to_string(),
        "national": p.format().mode(Mode::National).to_string(),
        "rfc3966": p.format().mode(Mode::Rfc3966).to_string(),
        "country_code": p.code().value(),
        "country": country,
        "national_number": p.national().value(),
        "valid": valid,
    })
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "phone"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Phone number parse / format / validate via the `phonenumber` crate (port of Google's \
         libphonenumber). Deterministic E.164 normalization, country detection, type \
         classification, and per-region display formatting. Pure local compute."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `phone_parse { number: \"(415) 555-1234\", default_country: \"US\" }` — normalize to E.164.\n\
             2. `phone_format { number: \"+14155551234\", style: \"international\" }` — display for a UI.\n\
             3. `phone_format { number: \"+14155551234\", style: \"rfc3966\" }` — generate a tel: URI.",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(PhoneParse), Box::new(PhoneFormat)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uk_international() {
        let p = parse(None, "+44 20 7946 0958").expect("parse");
        assert_eq!(p.code().value(), 44);
    }

    #[test]
    fn parses_us_with_default_country() {
        let p = parse(Some(CountryId::US), "(415) 555-1234").expect("parse");
        assert_eq!(p.code().value(), 1);
    }

    #[test]
    fn formats_e164() {
        let p = parse(Some(CountryId::US), "(415) 555-1234").unwrap();
        let s = p.format().mode(Mode::E164).to_string();
        assert_eq!(s, "+14155551234");
    }

    #[test]
    fn unknown_country_errors() {
        assert!(parse_country(Some("XX")).is_err());
    }
}
