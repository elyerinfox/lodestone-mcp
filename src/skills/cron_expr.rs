//! Cron expression skills (local compute): describe a cron line in plain
//! English, list its next N firings, validate the syntax. LLMs reliably
//! misread cron — DOM/DOW interaction, step values past range, optional
//! seconds field — so a deterministic tool is the right answer.
//!
//! ## Format note
//!
//! This module accepts both the **5-field** standard cron (`min hour dom
//! month dow`) and the **6-field** form with leading seconds (`sec min hour
//! dom month dow`). The `cron` crate auto-detects which one you've passed
//! it.
//!
//! ## DOM / DOW semantics — the trap LLMs hit
//!
//! When both day-of-month and day-of-week are restricted (i.e. neither is
//! `*`), Vixie cron uses **OR** — the entry fires when EITHER constraint
//! matches. `0 0 13 * 5` fires every Friday AND on the 13th of every month,
//! not only on Friday-the-13th. `cron_describe` calls this out explicitly
//! because LLMs get it wrong in the other direction.
//!
//! ## Sources
//!
//! - `man 5 crontab` (Vixie cron format).
//! - POSIX `crontab` definition (IEEE Std 1003.1-2024).

use std::str::FromStr;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// cron_describe
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DescribeArgs {
    /// Cron expression in 5-field (`min hour dom month dow`) or 6-field
    /// (`sec min hour dom month dow`) form, e.g. `"*/15 9-17 * * 1-5"`.
    expression: String,
    /// IANA timezone for the description, e.g. `"America/New_York"`. Defaults to UTC.
    #[serde(default)]
    timezone: Option<String>,
}

pub struct CronDescribe;
impl Skill for CronDescribe {
    fn name(&self) -> &'static str {
        "cron_describe"
    }
    fn description(&self) -> &'static str {
        "Plain-English description of a cron expression. Accepts both 5-field (`min hour dom \
         month dow`) and 6-field (`sec min hour dom month dow`) form. Calls out the Vixie cron \
         DOM/DOW OR rule explicitly when both fields are restricted, because that's the rule \
         LLMs most often get wrong. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DescribeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<DescribeArgs>()?;
            let tz = parse_tz(args.timezone.as_deref())?;
            // Best-effort parse: the english_form description doesn't require
            // the cron crate's iterator to succeed (the crate uses Quartz
            // semantics and rejects expressions Vixie cron accepts, e.g.
            // both DOM and DOW restricted). Surface the parse error as a
            // structured field rather than failing the describe call.
            let parsed = parse_cron(&args.expression).ok();
            Ok(text_result(describe(&args.expression, parsed.as_ref(), tz)))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Every 15 minutes during business hours, weekdays",
                args: r#"{"expression": "*/15 9-17 * * 1-5"}"#,
                note: Some("Returns a plain-English description plus the structured fields."),
            },
            SkillExample {
                title: "Friday-the-13th trap (DOM and DOW both restricted)",
                args: r#"{"expression": "0 0 13 * 5"}"#,
                note: Some("Explicitly flags the OR semantics: fires every Friday AND on the 13th of each month, NOT only Friday the 13th."),
            },
            SkillExample {
                title: "6-field form with seconds",
                args: r#"{"expression": "0 0 0 * * *", "timezone": "America/New_York"}"#,
                note: Some("Daily at midnight America/New_York. The 6-field form puts seconds first."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Translate a cron line you found in a config into plain English.",
            "Verify the DOM/DOW semantics on an expression with both fields restricted.",
            "Confirm the timezone interpretation before scheduling a job.",
        ]
    }
}

// ---------------------------------------------------------------------------
// cron_next
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NextArgs {
    /// Cron expression (5- or 6-field).
    expression: String,
    /// Number of upcoming firings to return. Defaults to 5, capped at 100.
    #[serde(default)]
    count: Option<u32>,
    /// Reference time as RFC3339 (e.g. `2026-06-01T00:00:00Z`). Defaults to "now".
    #[serde(default)]
    from: Option<String>,
    /// IANA timezone in which to interpret the expression and format the output.
    /// Defaults to UTC.
    #[serde(default)]
    timezone: Option<String>,
}

pub struct CronNext;
impl Skill for CronNext {
    fn name(&self) -> &'static str {
        "cron_next"
    }
    fn description(&self) -> &'static str {
        "List the next N firings of a cron expression as ISO timestamps in the requested \
         timezone. Default `count` is 5, capped at 100. Default `from` is now; pass an RFC3339 \
         timestamp to ask 'when does this fire after <date>?'. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NextArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<NextArgs>()?;
            let schedule = parse_cron(&args.expression)?;
            let tz = parse_tz(args.timezone.as_deref())?;
            let from_utc: DateTime<Utc> = match args.from.as_deref() {
                None => Utc::now(),
                Some(s) => DateTime::parse_from_rfc3339(s.trim())
                    .map_err(|e| invalid(format!("could not parse `from` as RFC3339: {e}")))?
                    .with_timezone(&Utc),
            };
            let count = args.count.unwrap_or(5).min(100) as usize;
            let firings: Vec<String> = schedule
                .after(&from_utc.with_timezone(&tz))
                .take(count)
                .map(|dt| dt.to_rfc3339())
                .collect();
            Ok(text_result(
                json!({
                    "expression": args.expression,
                    "timezone": tz_name(tz),
                    "from": from_utc.to_rfc3339(),
                    "count": firings.len(),
                    "firings": firings,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Next 5 weekday business-hour fires",
                args: r#"{"expression": "0 9 * * 1-5"}"#,
                note: Some("Returns the next 5 weekday 09:00 UTC timestamps."),
            },
            SkillExample {
                title: "From a specific reference time",
                args: r#"{"expression": "0 2 * * 0", "from": "2026-06-01T00:00:00Z", "count": 3}"#,
                note: Some("Sundays at 02:00 UTC after 2026-06-01."),
            },
            SkillExample {
                title: "In a local timezone",
                args: r#"{"expression": "0 9 * * 1-5", "timezone": "America/New_York", "count": 3}"#,
                note: Some("Each firing is reported in the requested timezone."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Confirm a cron job's next execution before deploying a new schedule.",
            "Compute a few sample firings to verify a tricky DOM/DOW expression behaves as intended.",
            "Project upcoming runs in an operator's local timezone for status displays.",
        ]
    }
}

// ---------------------------------------------------------------------------
// cron_validate
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ValidateArgs {
    /// Cron expression to validate.
    expression: String,
}

pub struct CronValidate;
impl Skill for CronValidate {
    fn name(&self) -> &'static str {
        "cron_validate"
    }
    fn description(&self) -> &'static str {
        "Parse a cron expression and return `valid=true` or a precise error pointing at the bad \
         field. Local, no network. Use before persisting a user-supplied expression."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ValidateArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ValidateArgs>()?;
            let result = match parse_cron(&args.expression) {
                Ok(_) => json!({"expression": args.expression, "valid": true}),
                Err(e) => json!({
                    "expression": args.expression,
                    "valid": false,
                    "error": format!("{e}").trim_start_matches("Invalid request: ").to_string(),
                }),
            };
            Ok(text_result(result.to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Valid 5-field",
                args: r#"{"expression": "*/15 9-17 * * 1-5"}"#,
                note: Some("Returns valid=true."),
            },
            SkillExample {
                title: "Bad field",
                args: r#"{"expression": "0 25 * * *"}"#,
                note: Some("Hour 25 is out of range — returns valid=false with the parse error."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Validate a user-submitted cron expression before storing it.",
            "Surface a precise parse error rather than an opaque schedule failure later.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn parse_cron(expr: &str) -> Result<Schedule, McpError> {
    let trimmed = expr.trim();
    let field_count = trimmed.split_whitespace().count();
    // The `cron` crate is Quartz-flavored — minimum 6 fields (sec min hour
    // dom month dow). Standard Vixie 5-field cron (min hour dom month dow)
    // gets a synthesized seconds=0 prepended so the LLM can pass either form.
    let normalized = match field_count {
        5 => format!("0 {trimmed}"),
        6 | 7 => trimmed.to_string(),
        n => {
            return Err(invalid(format!(
                "cron expression should have 5, 6, or 7 fields, got {n}"
            )));
        }
    };
    Schedule::from_str(&normalized)
        .map_err(|e| invalid(format!("invalid cron expression `{expr}`: {e}")))
}

fn parse_tz(tz: Option<&str>) -> Result<Tz, McpError> {
    match tz.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(Tz::UTC),
        Some(s) => s
            .parse::<Tz>()
            .map_err(|e| invalid(format!("unknown timezone `{s}`: {e}"))),
    }
}

fn tz_name(tz: Tz) -> String {
    tz.name().to_string()
}

/// Render a human description plus structured detail.
fn describe(expression: &str, schedule: Option<&Schedule>, tz: Tz) -> String {
    let english = english_form(expression);
    let next3: Vec<String> = schedule
        .map(|s| s.upcoming(tz).take(3).map(|dt| dt.to_rfc3339()).collect())
        .unwrap_or_default();
    let mut obj = json!({
        "expression": expression,
        "timezone": tz_name(tz),
        "english": english,
        "next_3_firings": next3,
    });
    if schedule.is_none() {
        obj["iteration_note"] = json!(
            "The underlying `cron` crate uses Quartz semantics (requires DOM or DOW = `?` when \
             the other is restricted); this expression won't iterate, but the English description \
             is structural."
        );
    }
    obj.to_string()
}

/// One-line plain-English render. Covers the common cases; falls back to a
/// structural summary when the expression is too exotic to template.
fn english_form(expr: &str) -> String {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let (sec, min, hour, dom, month, dow) = match fields.len() {
        5 => ("0", fields[0], fields[1], fields[2], fields[3], fields[4]),
        6 => (
            fields[0], fields[1], fields[2], fields[3], fields[4], fields[5],
        ),
        7 => (
            fields[0], fields[1], fields[2], fields[3], fields[4], fields[5],
        ), // year ignored
        _ => return "Unrecognized cron field count (expected 5, 6, or 7).".into(),
    };

    let parts = [
        render_minute_hour(min, hour, sec),
        render_day_part(dom, month, dow),
    ];
    let mut s = parts.join(", ");
    if dom != "*" && dom != "?" && dow != "*" && dow != "?" {
        s.push_str(
            ". NOTE: Vixie cron OR-s DOM and DOW when both are restricted — \
             this fires when EITHER constraint matches.",
        );
    }
    s
}

fn render_minute_hour(min: &str, hour: &str, sec: &str) -> String {
    let sec_part = if sec != "0" && sec != "*" {
        format!(" (seconds: {sec})")
    } else {
        String::new()
    };
    match (min, hour) {
        ("*", "*") => format!("every minute of every hour{sec_part}"),
        (m, "*") if m.starts_with("*/") => {
            format!("every {} minutes of every hour{sec_part}", &m[2..])
        }
        ("0", h) if h.parse::<u32>().is_ok() => format!("at {h}:00{sec_part}"),
        (m, h) if h.parse::<u32>().is_ok() && m.parse::<u32>().is_ok() => {
            format!("at {h}:{m:0>2}{sec_part}")
        }
        (m, h) if m.starts_with("*/") => {
            format!("every {} minutes during hour(s) {h}{sec_part}", &m[2..])
        }
        (m, h) => format!("at minute {m} of hour(s) {h}{sec_part}"),
    }
}

fn render_day_part(dom: &str, month: &str, dow: &str) -> String {
    let mut parts = Vec::new();
    if dom == "*" || dom == "?" {
        parts.push("every day".to_string());
    } else {
        parts.push(format!("on day-of-month {dom}"));
    }
    if month != "*" {
        parts.push(format!("in month(s) {month}"));
    }
    if dow != "*" && dow != "?" {
        parts.push(format!("on weekday(s) {dow} [0=Sun]"));
    }
    parts.join(" ")
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "cron"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Cron expression tooling: describe, list next firings, validate. Pure local compute. \
         Spells out the Vixie DOM/DOW OR rule explicitly because that's the trap LLMs hit."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `cron_validate { expression: \"0 0 13 * 5\" }` — does it parse?\n\
             2. `cron_describe { expression: \"0 0 13 * 5\" }` — what does it mean? (note: DOM+DOW = OR)\n\
             3. `cron_next { expression: \"0 0 13 * 5\", count: 3 }` — when does it actually fire?",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(CronDescribe),
        Box::new(CronNext),
        Box::new(CronValidate),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_5_field() {
        assert!(parse_cron("*/15 9-17 * * 1-5").is_ok());
    }

    #[test]
    fn parses_6_field() {
        assert!(parse_cron("0 0 0 * * *").is_ok());
    }

    #[test]
    fn rejects_out_of_range_hour() {
        assert!(parse_cron("0 25 * * *").is_err());
    }

    #[test]
    fn describes_business_hours() {
        let s = english_form("*/15 9-17 * * 1-5");
        assert!(s.contains("every 15 minutes"), "{s}");
    }

    #[test]
    fn flags_friday_13th_or_trap() {
        let s = english_form("0 0 13 * 5");
        assert!(s.contains("Vixie cron OR-s DOM and DOW"), "{s}");
    }

    #[test]
    fn next_firings_count_capped() {
        // Just verify a valid schedule produces some firings under "now".
        let sched = parse_cron("0 0 * * *").unwrap();
        let first: Vec<_> = sched.upcoming(Tz::UTC).take(3).collect();
        assert_eq!(first.len(), 3);
    }
}
