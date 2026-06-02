//! Date/time skills — `datetime`, `date_diff`, `time_convert`. The model's
//! training data has no current time, so these read the system clock and do
//! timezone/interval math (chrono + chrono-tz). Pure local computation, no network.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

/// Parse an *absolute* instant: a Unix timestamp or an RFC3339 string that
/// carries an offset. Returns `None` for tz-less (naive) inputs.
fn parse_instant(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{DateTime, TimeZone, Utc};
    let s = s.trim();
    if let Ok(ts) = s.parse::<i64>() {
        return Utc.timestamp_opt(ts, 0).single();
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

/// Parse a tz-less date/time: `YYYY-MM-DD[ T]HH:MM[:SS]` or a bare `YYYY-MM-DD`.
fn parse_naive(s: &str) -> Option<chrono::NaiveDateTime> {
    use chrono::{NaiveDate, NaiveDateTime};
    let s = s.trim();
    for fmt in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(ndt);
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
}

/// Parse any supported date/time to UTC, treating tz-less inputs as UTC.
pub(crate) fn parse_dt(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{TimeZone, Utc};
    parse_instant(s).or_else(|| parse_naive(s).map(|n| Utc.from_utc_datetime(&n)))
}

/// Parse an IANA timezone name (e.g. `America/New_York`, `Asia/Tokyo`, `UTC`).
fn parse_tz(name: &str) -> Option<chrono_tz::Tz> {
    name.trim().parse::<chrono_tz::Tz>().ok()
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DatetimeArgs {
    /// Optional IANA timezone (e.g. "America/New_York", "Asia/Tokyo", "UTC") to
    /// also show the current time in. Omit for just local + UTC.
    #[serde(default)]
    timezone: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DateDiffArgs {
    /// First date/time: ISO `YYYY-MM-DD`, RFC3339 (`2025-05-27T18:25:00Z`), or a
    /// Unix timestamp (seconds).
    from: String,
    /// Second date/time (same formats). Omit to compare against now.
    #[serde(default)]
    to: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TimeConvertArgs {
    /// The time to convert: ISO `YYYY-MM-DD[ T]HH:MM[:SS]`, RFC3339 (with offset),
    /// a bare `YYYY-MM-DD`, or a Unix timestamp.
    time: String,
    /// Target IANA timezone (e.g. "Asia/Tokyo", "America/Los_Angeles", "UTC").
    to_tz: String,
    /// Source IANA timezone for inputs that carry NO offset (default "UTC").
    /// Ignored when the input already has an offset or is a Unix timestamp.
    #[serde(default)]
    from_tz: Option<String>,
}

pub struct Datetime;
impl Skill for Datetime {
    fn name(&self) -> &'static str {
        "datetime"
    }
    fn description(&self) -> &'static str {
        "Get the current date and time from the system clock — local time (with UTC offset), UTC, \
        and the Unix timestamp. Use whenever you need to know 'now'; the model's training data has \
        no current time."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DatetimeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<DatetimeArgs>()?;
            use chrono::{Local, SecondsFormat, Utc};
            let local = Local::now();
            let utc = Utc::now();
            let mut out = format!(
                "Current date/time:\n  Local: {} ({})\n  UTC:   {}\n  Unix:  {}",
                local.to_rfc3339_opts(SecondsFormat::Secs, false),
                local.format("%A"),
                utc.to_rfc3339_opts(SecondsFormat::Secs, true),
                utc.timestamp(),
            );
            if let Some(name) = args
                .timezone
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let tz = parse_tz(name).ok_or_else(|| {
                    invalid(format!(
                        "unknown timezone '{name}' (use an IANA name like America/New_York)"
                    ))
                })?;
                out.push_str(&format!(
                    "\n  {name}: {}",
                    utc.with_timezone(&tz)
                        .to_rfc3339_opts(SecondsFormat::Secs, false)
                ));
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Now in local + UTC",
                args: r#"{}"#,
                note: Some("Returns local time, UTC, day-of-week, and Unix timestamp."),
            },
            SkillExample {
                title: "Also show Tokyo time",
                args: r#"{"timezone": "Asia/Tokyo"}"#,
                note: Some("Add any IANA zone name to also print 'now' there."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Ground the model with the current date/time before any temporal reasoning.",
            "Get the current time in a specific IANA timezone.",
            "Look up today's Unix timestamp.",
        ]
    }
}

pub struct DateDiff;
impl Skill for DateDiff {
    fn name(&self) -> &'static str {
        "date_diff"
    }
    fn description(&self) -> &'static str {
        "Compute the difference between two dates/times: days (and approximate years), hours, and a \
        human 'ago / from now'. Accepts ISO YYYY-MM-DD, RFC3339, or a Unix timestamp; `to` defaults \
        to now. Use to judge recency — e.g. how long ago a release came out."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DateDiffArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<DateDiffArgs>()?;
            use chrono::{SecondsFormat, Utc};
            let from = parse_dt(&args.from)
                .ok_or_else(|| invalid(format!("could not parse date/time: '{}'", args.from)))?;
            let to_str = args.to.as_deref().map(str::trim).filter(|s| !s.is_empty());
            let to = match to_str {
                Some(s) => parse_dt(s)
                    .ok_or_else(|| invalid(format!("could not parse date/time: '{s}'")))?,
                None => Utc::now(),
            };
            let diff = to - from;
            let days = diff.num_days();
            let mut out = format!(
                "{}  →  {}\n  {days} days",
                from.to_rfc3339_opts(SecondsFormat::Secs, true),
                to.to_rfc3339_opts(SecondsFormat::Secs, true),
            );
            if days.abs() >= 365 {
                out.push_str(&format!(" (~{:.1} years)", days.abs() as f64 / 365.25));
            }
            out.push_str(&format!("\n  {} hours", diff.num_hours()));
            if to_str.is_none() {
                let phrase = match days.cmp(&0) {
                    std::cmp::Ordering::Greater => format!("{days} day(s) ago"),
                    std::cmp::Ordering::Less => format!("{} day(s) from now", -days),
                    std::cmp::Ordering::Equal => "today".to_string(),
                };
                out.push_str(&format!("\n  → that date is {phrase}"));
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "How long ago was this date?",
                args: r#"{"from": "2020-01-15"}"#,
                note: Some("Omit `to` to compare against now."),
            },
            SkillExample {
                title: "Days between two dates",
                args: r#"{"from": "2024-01-01", "to": "2024-12-31"}"#,
                note: None,
            },
            SkillExample {
                title: "Hours between RFC3339 instants",
                args: r#"{"from": "2025-05-27T18:25:00Z", "to": "2025-05-28T06:00:00Z"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Judge recency of a release, event, or document.",
            "Compute exact days/hours between two dates.",
            "Verify a model claim about how long ago something happened.",
        ]
    }
}

pub struct TimeConvert;
impl Skill for TimeConvert {
    fn name(&self) -> &'static str {
        "time_convert"
    }
    fn description(&self) -> &'static str {
        "Convert a date/time to another timezone. Accepts ISO/RFC3339, a bare YYYY-MM-DD, or a Unix \
        timestamp; `to_tz` is the target IANA zone (e.g. Asia/Tokyo). For inputs without an offset, \
        `from_tz` says how to interpret them (default UTC)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TimeConvertArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<TimeConvertArgs>()?;
            use chrono::{SecondsFormat, TimeZone, Utc};
            let to_tz = parse_tz(&args.to_tz)
                .ok_or_else(|| invalid(format!("unknown timezone '{}'", args.to_tz)))?;
            let instant = match parse_instant(&args.time) {
                Some(utc) => utc,
                None => {
                    let naive = parse_naive(&args.time)
                        .ok_or_else(|| invalid(format!("could not parse time: '{}'", args.time)))?;
                    let from_tz = match args
                        .from_tz
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        Some(s) => {
                            parse_tz(s).ok_or_else(|| invalid(format!("unknown timezone '{s}'")))?
                        }
                        None => chrono_tz::UTC,
                    };
                    from_tz
                        .from_local_datetime(&naive)
                        .single()
                        .ok_or_else(|| {
                            invalid("that local time is ambiguous or invalid in from_tz")
                        })?
                        .with_timezone(&Utc)
                }
            };
            let out = format!(
                "{}: {}\nUTC: {}",
                args.to_tz.trim(),
                instant
                    .with_timezone(&to_tz)
                    .to_rfc3339_opts(SecondsFormat::Secs, false),
                instant.to_rfc3339_opts(SecondsFormat::Secs, true),
            );
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "NY meeting time in Tokyo",
                args: r#"{"time": "2025-06-15 09:00", "to_tz": "Asia/Tokyo", "from_tz": "America/New_York"}"#,
                note: None,
            },
            SkillExample {
                title: "RFC3339 instant to LA",
                args: r#"{"time": "2025-05-27T18:25:00Z", "to_tz": "America/Los_Angeles"}"#,
                note: Some("With an offset present, `from_tz` is ignored."),
            },
            SkillExample {
                title: "Unix timestamp to Berlin",
                args: r#"{"time": "1716835200", "to_tz": "Europe/Berlin"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert a meeting time across timezones.",
            "Render a UTC instant in a user's local zone.",
            "Translate a Unix timestamp into a human-readable zoned time.",
        ]
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(Datetime),
        Box::new(DateDiff),
        Box::new(TimeConvert),
    ]
}

#[cfg(test)]
mod tests {
    use super::parse_dt;

    #[test]
    fn parse_dt_accepts_common_formats() {
        let d = parse_dt("2025-05-27").unwrap();
        assert_eq!(
            d.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "2025-05-27T00:00:00Z"
        );
        let d = parse_dt("2025-05-27T18:25:00-07:00").unwrap();
        assert_eq!(
            d.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "2025-05-28T01:25:00Z"
        );
        assert_eq!(parse_dt("0").unwrap().timestamp(), 0);
        assert!(parse_dt("not a date").is_none());
    }
}
