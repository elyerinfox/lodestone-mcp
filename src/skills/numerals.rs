//! Numeral system / number-name skills (local compute): base conversion
//! (any base 2-36), Roman numerals, English number-to-words. Pure-Rust.
//! LLMs make small but consistent arithmetic errors on these — wrong
//! Roman ordering, off-by-one in base conversions, scale confusion
//! (long vs short, "billion") in number-to-words.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// ---------- base conversion ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BaseArgs {
    /// Number as a string in `from_base`.
    number: String,
    /// Source base (2-36).
    from_base: u32,
    /// Target base (2-36).
    to_base: u32,
}

pub struct NumeralsBaseConvert;
impl Skill for NumeralsBaseConvert {
    fn name(&self) -> &'static str {
        "numerals_base_convert"
    }
    fn description(&self) -> &'static str {
        "Convert a number between any two bases from 2 to 36. Negative numbers OK. Returns the result lowercased; for hex output use lowercase a-f."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BaseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<BaseArgs>()?;
            let raw = a.number.trim();
            let (neg, body) = if let Some(rest) = raw.strip_prefix('-') {
                (true, rest)
            } else {
                (false, raw)
            };
            // Strip a 0x / 0b / 0o prefix if it matches the from_base.
            let body = match a.from_base {
                16 => body
                    .strip_prefix("0x")
                    .or_else(|| body.strip_prefix("0X"))
                    .unwrap_or(body),
                2 => body
                    .strip_prefix("0b")
                    .or_else(|| body.strip_prefix("0B"))
                    .unwrap_or(body),
                8 => body
                    .strip_prefix("0o")
                    .or_else(|| body.strip_prefix("0O"))
                    .unwrap_or(body),
                _ => body,
            };
            let n = i128::from_str_radix(body, a.from_base).map_err(|e| {
                invalid(format!(
                    "could not parse `{body}` as base {}: {e}",
                    a.from_base
                ))
            })?;
            let mag = if neg { -n } else { n };
            let s = render_in_base(mag.unsigned_abs(), a.to_base);
            let out = if mag < 0 { format!("-{s}") } else { s };
            Ok(text_result(
                json!({"converted": out, "from_base": a.from_base, "to_base": a.to_base})
                    .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Decimal -> hex",
                args: r#"{"number": "255", "from_base": 10, "to_base": 16}"#,
                note: Some("Returns `ff`."),
            },
            SkillExample {
                title: "Binary -> decimal",
                args: r#"{"number": "101101", "from_base": 2, "to_base": 10}"#,
                note: Some("Returns `45`."),
            },
            SkillExample {
                title: "Hex with prefix",
                args: r#"{"number": "0xCAFE", "from_base": 16, "to_base": 10}"#,
                note: Some("`0x` is stripped; returns `51966`."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert between hex / decimal / binary / octal without mental arithmetic.",
            "Decode an arbitrary-base number from a homework / CTF problem.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "from_base",
                min: Some(2.0),
                max: Some(36.0),
            },
            Rule::Range {
                field: "to_base",
                min: Some(2.0),
                max: Some(36.0),
            },
            Rule::Length {
                field: "number",
                min: Some(1),
                max: None,
            },
        ]
    }
}

fn render_in_base(n: u128, base: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut out = String::new();
    let mut n = n;
    while n > 0 {
        let d = (n % base as u128) as u32;
        out.push(std::char::from_digit(d, base).unwrap());
        n /= base as u128;
    }
    out.chars().rev().collect()
}

// ---------- Roman numerals ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RomanArgs {
    /// Either a positive integer 1-3999 (as a number) or a Roman numeral string. Direction is
    /// inferred from the input type.
    value: serde_json::Value,
}

pub struct NumeralsRoman;
impl Skill for NumeralsRoman {
    fn name(&self) -> &'static str {
        "numerals_roman"
    }
    fn description(&self) -> &'static str {
        "Convert between Arabic and Roman numerals (1-3999). Integer input -> Roman string; Roman string input -> integer. Validates the input direction."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RomanArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<RomanArgs>()?;
            let out = match a.value {
                serde_json::Value::Number(n) => {
                    let i = n.as_i64().ok_or_else(|| invalid("non-integer number"))?;
                    if !(1..=3999).contains(&i) {
                        return Err(invalid("Roman supports 1..=3999"));
                    }
                    json!({"arabic": i, "roman": to_roman(i as u32)})
                }
                serde_json::Value::String(s) => {
                    let n = from_roman(s.trim().to_ascii_uppercase().as_str())?;
                    json!({"roman": s, "arabic": n})
                }
                _ => return Err(invalid("value must be a number or string")),
            };
            Ok(text_result(out.to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Number -> Roman",
                args: r#"{"value": 1994}"#,
                note: Some("Returns `MCMXCIV`."),
            },
            SkillExample {
                title: "Roman -> number",
                args: r#"{"value": "MMXXVI"}"#,
                note: Some("Returns `2026`."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &["Compute a Roman numeral without subtractive-rule mistakes (LLMs frequently emit `IIII` for 4)."]
    }
}

fn to_roman(mut n: u32) -> String {
    const PAIRS: &[(u32, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (v, r) in PAIRS {
        while n >= *v {
            s.push_str(r);
            n -= *v;
        }
    }
    s
}

fn from_roman(s: &str) -> Result<u32, McpError> {
    let val = |c: char| match c {
        'I' => Some(1),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        'D' => Some(500),
        'M' => Some(1000),
        _ => None,
    };
    let mut total: i32 = 0;
    let mut prev: i32 = 0;
    for c in s.chars().rev() {
        let v = val(c).ok_or_else(|| invalid(format!("`{c}` isn't a Roman digit")))?;
        if v < prev {
            total -= v;
        } else {
            total += v;
            prev = v;
        }
    }
    if !(1..=3999).contains(&total) {
        return Err(invalid("Roman value out of range 1..=3999"));
    }
    // Sanity-check by re-encoding.
    if to_roman(total as u32) != s {
        return Err(invalid(format!(
            "`{s}` is not canonical Roman; canonical form would be `{}`",
            to_roman(total as u32)
        )));
    }
    Ok(total as u32)
}

// ---------- Number to English words ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WordsArgs {
    /// Integer to spell out (-10^15 < n < 10^15).
    number: i64,
    /// `short` (default — US/UK modern) or `long` (older British "milliard").
    #[serde(default)]
    scale: Option<String>,
}

pub struct NumeralsToWords;
impl Skill for NumeralsToWords {
    fn name(&self) -> &'static str {
        "numerals_to_words"
    }
    fn description(&self) -> &'static str {
        "Spell out an integer in English. `scale=short` (default) uses billion=10^9 / trillion=10^12 etc.; `scale=long` uses milliard=10^9 / billion=10^12. Range -10^15 < n < 10^15."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WordsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<WordsArgs>()?;
            let scale = a.scale.as_deref().unwrap_or("short");
            let words = num_to_words(a.number, scale == "long");
            Ok(text_result(
                json!({"number": a.number, "scale": scale, "words": words}).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Common case",
                args: r#"{"number": 1234567}"#,
                note: Some(
                    "`one million two hundred thirty-four thousand five hundred sixty-seven`.",
                ),
            },
            SkillExample {
                title: "Long scale (British)",
                args: r#"{"number": 1000000000, "scale": "long"}"#,
                note: Some("`one milliard` instead of `one billion`."),
            },
            SkillExample {
                title: "Negative",
                args: r#"{"number": -42}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Spell out an amount accurately for legal / financial text.",
            "Disambiguate short vs long scale (billion/milliard) without guessing.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::OneOf {
            field: "scale",
            values: &["short", "long"],
        }]
    }
}

fn num_to_words(n: i64, long_scale: bool) -> String {
    if n == 0 {
        return "zero".into();
    }
    let mut s = String::new();
    let mut n = n;
    if n < 0 {
        s.push_str("negative ");
        n = -n;
    }
    let scale_names: &[&str] = if long_scale {
        &["", " thousand", " million", " milliard", " billion"]
    } else {
        &["", " thousand", " million", " billion", " trillion"]
    };
    let mut parts: Vec<String> = Vec::new();
    let mut idx = 0;
    while n > 0 && idx < scale_names.len() {
        let group = (n % 1000) as u32;
        if group > 0 {
            let g = group_to_words(group);
            parts.push(format!("{g}{}", scale_names[idx]));
        }
        n /= 1000;
        idx += 1;
    }
    parts.reverse();
    s.push_str(&parts.join(" "));
    s
}

fn group_to_words(n: u32) -> String {
    let ones = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    let tens = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    let h = n / 100;
    let r = n % 100;
    let mut s = String::new();
    if h > 0 {
        s.push_str(ones[h as usize]);
        s.push_str(" hundred");
        if r > 0 {
            s.push(' ');
        }
    }
    if r >= 20 {
        s.push_str(tens[(r / 10) as usize]);
        if !r.is_multiple_of(10) {
            s.push('-');
            s.push_str(ones[(r % 10) as usize]);
        }
    } else if r > 0 {
        s.push_str(ones[r as usize]);
    }
    s
}

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "numerals"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Numeral system tools: base conversion (any 2-36), Roman numerals (1-3999), English number-to-words (short / long scale). Pure local compute."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some("1. `numerals_base_convert { number: \"255\", from_base: 10, to_base: 16 }` — hex view.\n2. `numerals_roman { value: 2026 }` — Roman form.\n3. `numerals_to_words { number: 1234 }` — spell out.")
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(NumeralsBaseConvert),
        Box::new(NumeralsRoman),
        Box::new(NumeralsToWords),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dec_to_hex() {
        assert_eq!(render_in_base(255, 16), "ff");
    }
    #[test]
    fn bin_to_dec() {
        assert_eq!(i128::from_str_radix("101101", 2).unwrap(), 45);
    }
    #[test]
    fn roman_round_trip() {
        assert_eq!(to_roman(1994), "MCMXCIV");
        assert_eq!(from_roman("MCMXCIV").unwrap(), 1994);
    }
    #[test]
    fn roman_rejects_iiii() {
        assert!(from_roman("IIII").is_err());
    }
    #[test]
    fn words_small() {
        assert_eq!(num_to_words(0, false), "zero");
        assert_eq!(num_to_words(42, false), "forty-two");
    }
    #[test]
    fn words_million() {
        assert_eq!(
            num_to_words(1_234_567, false),
            "one million two hundred thirty-four thousand five hundred sixty-seven"
        );
    }
    #[test]
    fn long_scale_milliard() {
        assert_eq!(num_to_words(1_000_000_000, true), "one milliard");
    }
}
