//! Duration skills (local compute): parse human-style ("2h 30m" / "1d4h12s")
//! AND ISO 8601 durations ("PT2H30M" / "P3DT4H"), format seconds in human form,
//! compute the span between two RFC3339 timestamps. Pure-Rust via the
//! existing `chrono` dep.

use std::sync::Arc;

use chrono::DateTime;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// ---------- parse ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ParseArgs {
    /// Duration string. Accepts ISO 8601 ("PT2H30M", "P1DT12H"), human-friendly
    /// ("2h 30m", "1d4h12s", "90s", "1.5h"), or a bare number (interpreted as seconds).
    duration: String,
}

pub struct DurationParse;
impl Skill for DurationParse {
    fn name(&self) -> &'static str {
        "duration_parse"
    }
    fn description(&self) -> &'static str {
        "Parse a duration string into total seconds (i64) plus its broken-down form (days/hours/minutes/seconds). Accepts ISO 8601 (`PT2H30M`), human-friendly (`2h 30m`, `1d 4h 12s`, `90s`, `1.5h`), or a bare number (seconds)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ParseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ParseArgs>()?;
            let total = parse_any(&a.duration)?;
            let (d, h, m, s) = breakdown(total);
            Ok(text_result(
                json!({
                    "input": a.duration,
                    "total_seconds": total,
                    "days": d,
                    "hours": h,
                    "minutes": m,
                    "seconds": s,
                    "iso8601": to_iso8601(total),
                    "human": format_human(total),
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Human",
                args: r#"{"duration": "2h 30m"}"#,
                note: Some("Returns 9000 seconds, ISO `PT2H30M`."),
            },
            SkillExample {
                title: "ISO 8601",
                args: r#"{"duration": "P1DT4H30M"}"#,
                note: None,
            },
            SkillExample {
                title: "Bare seconds",
                args: r#"{"duration": "3600"}"#,
                note: Some("Returns 1h."),
            },
            SkillExample {
                title: "Fractional hours",
                args: r#"{"duration": "1.5h"}"#,
                note: Some("Treated as 5400 seconds."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Normalize a user-entered duration to seconds.",
            "Round-trip ISO 8601 durations from a configuration field.",
        ]
    }
}

fn parse_any(s: &str) -> Result<i64, McpError> {
    let raw = s.trim();
    if raw.is_empty() {
        return Err(invalid("empty duration"));
    }
    // Bare number — seconds.
    if let Ok(n) = raw.parse::<i64>() {
        return Ok(n);
    }
    if let Ok(n) = raw.parse::<f64>() {
        return Ok(n.round() as i64);
    }
    // ISO 8601 — starts with P.
    if raw.starts_with('P') || raw.starts_with('p') {
        return parse_iso8601(raw);
    }
    // Human-friendly: tokens like 1d, 2h, 30m, 45s, 1.5h.
    parse_human(raw)
}

fn parse_iso8601(s: &str) -> Result<i64, McpError> {
    // Format: P[nD]T[nH][nM][nS]. We support the common case (no weeks/months/years).
    let upper = s.to_ascii_uppercase();
    let rest = upper.trim_start_matches('P');
    let (date_part, time_part) = match rest.split_once('T') {
        Some((d, t)) => (d, t),
        None => (rest, ""),
    };
    let mut total = 0i64;
    // Date part: only days supported deterministically (weeks need definition; months/years are calendar-dependent).
    if !date_part.is_empty() {
        if let Some(d_str) = date_part.strip_suffix('D') {
            let d: f64 = d_str
                .parse()
                .map_err(|_| invalid(format!("could not parse `{date_part}` as days")))?;
            total += (d * 86400.0) as i64;
        } else if let Some(w_str) = date_part.strip_suffix('W') {
            let w: f64 = w_str
                .parse()
                .map_err(|_| invalid(format!("could not parse `{date_part}` as weeks")))?;
            total += (w * 86400.0 * 7.0) as i64;
        } else {
            return Err(invalid(format!(
                "unsupported date component `{date_part}` (only D / W supported, not Y or M)"
            )));
        }
    }
    if !time_part.is_empty() {
        // Walk tokens H / M / S.
        let mut buf = String::new();
        for c in time_part.chars() {
            if c.is_ascii_digit() || c == '.' {
                buf.push(c);
                continue;
            }
            let v: f64 = buf
                .parse()
                .map_err(|_| invalid(format!("bad number in `{time_part}`")))?;
            buf.clear();
            match c.to_ascii_uppercase() {
                'H' => total += (v * 3600.0) as i64,
                'M' => total += (v * 60.0) as i64,
                'S' => total += v as i64,
                _ => return Err(invalid(format!("unknown time component `{c}`"))),
            }
        }
        if !buf.is_empty() {
            return Err(invalid(format!(
                "trailing number `{buf}` in time component (missing H/M/S)"
            )));
        }
    }
    Ok(total)
}

fn parse_human(s: &str) -> Result<i64, McpError> {
    let mut total = 0i64;
    let lowered = s.to_ascii_lowercase();
    let bytes: Vec<char> = lowered.chars().collect();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if bytes[i].is_whitespace() || bytes[i] == ',' {
            i += 1;
            continue;
        }
        let mut num = String::new();
        while i < n && (bytes[i].is_ascii_digit() || bytes[i] == '.') {
            num.push(bytes[i]);
            i += 1;
        }
        if num.is_empty() {
            return Err(invalid(format!("expected a number near offset {i}")));
        }
        let val: f64 = num
            .parse()
            .map_err(|_| invalid(format!("bad number `{num}`")))?;
        // Skip whitespace.
        while i < n && bytes[i].is_whitespace() {
            i += 1;
        }
        // Read unit.
        let mut unit = String::new();
        while i < n && bytes[i].is_ascii_alphabetic() {
            unit.push(bytes[i]);
            i += 1;
        }
        let mult: f64 = match unit.as_str() {
            "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
            "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
            "d" | "day" | "days" => 86400.0,
            "w" | "wk" | "week" | "weeks" => 86400.0 * 7.0,
            "" => return Err(invalid(format!("`{val}` missing a unit (s/m/h/d/w)"))),
            other => return Err(invalid(format!("unknown unit `{other}` (try s/m/h/d/w)"))),
        };
        total += (val * mult) as i64;
    }
    Ok(total)
}

fn breakdown(total: i64) -> (i64, i64, i64, i64) {
    let mut t = total.abs();
    let d = t / 86400;
    t %= 86400;
    let h = t / 3600;
    t %= 3600;
    let m = t / 60;
    t %= 60;
    (if total < 0 { -d } else { d }, h, m, t)
}

fn to_iso8601(total: i64) -> String {
    if total == 0 {
        return "PT0S".into();
    }
    let neg = total < 0;
    let (d, h, m, s) = breakdown(total.abs());
    let mut out = String::from("P");
    if d > 0 {
        out.push_str(&format!("{d}D"));
    }
    let needs_t = h > 0 || m > 0 || s > 0;
    if needs_t {
        out.push('T');
    }
    if h > 0 {
        out.push_str(&format!("{h}H"));
    }
    if m > 0 {
        out.push_str(&format!("{m}M"));
    }
    if s > 0 {
        out.push_str(&format!("{s}S"));
    }
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

fn format_human(total: i64) -> String {
    if total == 0 {
        return "0s".into();
    }
    let neg = total < 0;
    let (d, h, m, s) = breakdown(total.abs());
    let mut parts: Vec<String> = Vec::new();
    if d > 0 {
        parts.push(format!("{d}d"));
    }
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 {
        parts.push(format!("{m}m"));
    }
    if s > 0 {
        parts.push(format!("{s}s"));
    }
    let joined = parts.join(" ");
    if neg {
        format!("-{joined}")
    } else {
        joined
    }
}

// ---------- format ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FormatArgs {
    /// Total seconds (positive or negative).
    seconds: i64,
    /// Format: `human` (default — `1d 4h 12m 5s`), `iso8601` (`PT...`), `hms` (`HH:MM:SS`).
    #[serde(default)]
    style: Option<String>,
}

pub struct DurationFormat;
impl Skill for DurationFormat {
    fn name(&self) -> &'static str {
        "duration_format"
    }
    fn description(&self) -> &'static str {
        "Format a duration given in seconds as `human` (default), `iso8601`, or `hms` (`HH:MM:SS`)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FormatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<FormatArgs>()?;
            let style = a.style.as_deref().unwrap_or("human");
            let out = match style {
                "human" => format_human(a.seconds),
                "iso8601" | "iso" => to_iso8601(a.seconds),
                "hms" => {
                    let hours = a.seconds.abs() / 3600;
                    let mins = (a.seconds.abs() / 60) % 60;
                    let secs = a.seconds.abs() % 60;
                    format!(
                        "{}{:02}:{:02}:{:02}",
                        if a.seconds < 0 { "-" } else { "" },
                        hours,
                        mins,
                        secs
                    )
                }
                _ => unreachable!("validation_rules restricts `style` to the matched arms"),
            };
            Ok(text_result(
                json!({"seconds": a.seconds, "style": style, "formatted": out}).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Human",
                args: r#"{"seconds": 9045}"#,
                note: Some("Returns `2h 30m 45s`."),
            },
            SkillExample {
                title: "ISO 8601",
                args: r#"{"seconds": 9045, "style": "iso8601"}"#,
                note: Some("Returns `PT2H30M45S`."),
            },
            SkillExample {
                title: "HH:MM:SS",
                args: r#"{"seconds": 9045, "style": "hms"}"#,
                note: Some("Returns `02:30:45`."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Render an interval in a chosen style for display.",
            "Round-trip seconds through ISO 8601 form.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::OneOf {
            field: "style",
            values: &["human", "iso8601", "iso", "hms"],
        }]
    }
}

// ---------- between ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BetweenArgs {
    /// Start time as RFC3339 (e.g. `2026-06-01T00:00:00Z`).
    start: String,
    /// End time as RFC3339.
    end: String,
}

pub struct DurationBetween;
impl Skill for DurationBetween {
    fn name(&self) -> &'static str {
        "duration_between"
    }
    fn description(&self) -> &'static str {
        "Compute the signed duration between two RFC3339 timestamps. Returns total seconds, broken-down components, and ISO 8601 / human / hms formats. Negative when `end` precedes `start`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BetweenArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<BetweenArgs>()?;
            let start = DateTime::parse_from_rfc3339(a.start.trim())
                .map_err(|e| invalid(format!("bad start: {e}")))?;
            let end = DateTime::parse_from_rfc3339(a.end.trim())
                .map_err(|e| invalid(format!("bad end: {e}")))?;
            let total = (end - start).num_seconds();
            let (d, h, m, s) = breakdown(total);
            Ok(text_result(
                json!({
                    "start": a.start,
                    "end": a.end,
                    "total_seconds": total,
                    "days": d,
                    "hours": h,
                    "minutes": m,
                    "seconds": s,
                    "iso8601": to_iso8601(total),
                    "human": format_human(total),
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Day between two dates",
                args: r#"{"start": "2026-06-01T00:00:00Z", "end": "2026-06-02T00:00:00Z"}"#,
                note: Some("Returns 86400 seconds, `P1D`."),
            },
            SkillExample {
                title: "Crossing timezones",
                args: r#"{"start": "2026-06-01T09:00:00-04:00", "end": "2026-06-01T15:00:00+00:00"}"#,
                note: Some("Times are normalized to UTC first."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute the elapsed time between two timestamps without timezone-arithmetic mistakes.",
            "Sanity-check an age / TTL / deadline interval.",
        ]
    }
}

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "duration"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Duration parsing (human / ISO 8601 / bare seconds), formatting (human / iso8601 / HH:MM:SS), and the span between two RFC3339 timestamps. Pure local compute via chrono."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some("1. `duration_parse { duration: \"2h 30m\" }` — normalize to seconds.\n2. `duration_format { seconds: 9000, style: \"iso8601\" }` — render as ISO.\n3. `duration_between { start: \"...\", end: \"...\" }` — span between two timestamps.")
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(DurationParse),
        Box::new(DurationFormat),
        Box::new(DurationBetween),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_human_simple() {
        assert_eq!(parse_human("2h 30m").unwrap(), 9000);
    }
    #[test]
    fn parse_human_compact() {
        assert_eq!(parse_human("1d4h12s").unwrap(), 86400 + 4 * 3600 + 12);
    }
    #[test]
    fn parse_iso_basic() {
        assert_eq!(parse_iso8601("PT2H30M").unwrap(), 9000);
    }
    #[test]
    fn parse_iso_with_day() {
        assert_eq!(parse_iso8601("P1DT4H").unwrap(), 86400 + 4 * 3600);
    }
    #[test]
    fn parse_bare_seconds() {
        assert_eq!(parse_any("3600").unwrap(), 3600);
    }
    #[test]
    fn iso8601_roundtrip() {
        assert_eq!(to_iso8601(9000), "PT2H30M");
        assert_eq!(to_iso8601(86400 + 4 * 3600), "P1DT4H");
    }
    #[test]
    fn human_format() {
        assert_eq!(format_human(9045), "2h 30m 45s");
    }
}
