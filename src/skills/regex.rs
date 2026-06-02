//! Regex skills (local, no network): `regex_search` finds matches (with capture
//! groups) and `regex_replace` substitutes. Uses the Rust `regex` crate syntax
//! (no look-around/backrefs); `$1` / `${name}` in replacements.

use std::sync::Arc;

use futures::future::BoxFuture;
use regex::RegexBuilder;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

const MAX_MATCHES: usize = 200;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RegexSearchArgs {
    /// The regular expression (Rust `regex` syntax).
    pattern: String,
    /// The text to search.
    text: String,
    /// Find all matches (default true). When false, only the first match.
    #[serde(default)]
    all: Option<bool>,
    /// Case-insensitive matching (default false).
    #[serde(default)]
    ignore_case: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RegexReplaceArgs {
    /// The regular expression (Rust `regex` syntax).
    pattern: String,
    /// The text to transform.
    text: String,
    /// Replacement; supports `$1` / `${name}` group references.
    replacement: String,
    /// Replace all matches (default true). When false, only the first.
    #[serde(default)]
    all: Option<bool>,
    /// Case-insensitive matching (default false).
    #[serde(default)]
    ignore_case: Option<bool>,
}

fn build(pattern: &str, ignore_case: bool) -> Result<regex::Regex, McpError> {
    RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|e| invalid(format!("invalid regex: {e}")))
}

pub struct RegexSearch;
impl Skill for RegexSearch {
    fn name(&self) -> &'static str {
        "regex_search"
    }
    fn description(&self) -> &'static str {
        "Search text with a regular expression (Rust `regex` syntax; no look-around/backrefs). \
        Returns each match and its capture groups (numbered and named). Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RegexSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<RegexSearchArgs>()?;
            let re = build(&args.pattern, args.ignore_case.unwrap_or(false))?;
            let all = args.all.unwrap_or(true);
            let names: Vec<Option<&str>> = re.capture_names().collect();

            let mut out = String::new();
            let mut count = 0usize;
            for caps in re
                .captures_iter(&args.text)
                .take(if all { MAX_MATCHES } else { 1 })
            {
                count += 1;
                let whole = caps.get(0).map(|m| m.as_str()).unwrap_or("");
                out.push_str(&format!("\nMatch {count}: {whole}"));
                for (i, name) in names.iter().enumerate().skip(1) {
                    if let Some(g) = caps.get(i) {
                        match name {
                            Some(n) => out.push_str(&format!("\n  ${i} ({n}): {}", g.as_str())),
                            None => out.push_str(&format!("\n  ${i}: {}", g.as_str())),
                        }
                    }
                }
                out.push('\n');
            }
            if count == 0 {
                return Ok(text_result("No matches."));
            }
            Ok(text_result(format!("{count} match(es):\n{out}")))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Find all email-like tokens",
                args: r#"{"pattern": "[\\w.+-]+@[\\w.-]+", "text": "ping alice@example.com and bob@x.io"}"#,
                note: Some("Returns each match; capture groups numbered and named (if any)."),
            },
            SkillExample {
                title: "Case-insensitive single match",
                args: r#"{"pattern": "error: (?P<msg>.+)", "text": "ERROR: disk full", "all": false, "ignore_case": true}"#,
                note: Some("Stops at the first hit; named capture `msg` is reported."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Extract structured fields from log lines or pasted output.",
            "Verify whether some pattern appears anywhere in a blob of text.",
            "Pull named capture groups for downstream processing.",
        ]
    }
}

pub struct RegexReplace;
impl Skill for RegexReplace {
    fn name(&self) -> &'static str {
        "regex_replace"
    }
    fn description(&self) -> &'static str {
        "Replace regex matches in text. `replacement` supports `$1` / `${name}` group references. \
        Replaces all matches by default (`all=false` for just the first). Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RegexReplaceArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<RegexReplaceArgs>()?;
            let re = build(&args.pattern, args.ignore_case.unwrap_or(false))?;
            let out = if args.all.unwrap_or(true) {
                re.replace_all(&args.text, args.replacement.as_str())
                    .into_owned()
            } else {
                re.replacen(&args.text, 1, args.replacement.as_str())
                    .into_owned()
            };
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Swap order of two captures",
                args: r#"{"pattern": "(\\w+) (\\w+)", "text": "alice bob", "replacement": "$2 $1"}"#,
                note: Some("Returns `bob alice`."),
            },
            SkillExample {
                title: "Replace first match only, case-insensitive",
                args: r#"{"pattern": "foo", "text": "Foo and foo", "replacement": "bar", "all": false, "ignore_case": true}"#,
                note: Some("Returns `bar and foo`."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Normalize whitespace or punctuation across a text blob.",
            "Rewrite log lines into a different field order.",
            "Strip or anonymize sensitive tokens (emails, IDs) before sharing.",
        ]
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(RegexSearch), Box::new(RegexReplace)]
}
