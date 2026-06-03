//! Text manipulation skills (local compute): case conversion (snake/kebab/
//! camel/pascal/screaming/sentence/title), slugify, Levenshtein edit
//! distance, line diff, word/char counts. Pure-Rust, no external crate.
//! LLMs miss case-conversion edge cases (acronyms, leading digits) and
//! make arithmetic errors on edit distance.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// ---------- case convert ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CaseArgs {
    /// Input text.
    text: String,
    /// Target case: `snake`, `kebab`, `camel`, `pascal`, `screaming` (SCREAMING_SNAKE),
    /// `screaming_kebab` (SCREAMING-KEBAB), `sentence`, `title`.
    target: String,
}

pub struct TextCaseConvert;
impl Skill for TextCaseConvert {
    fn name(&self) -> &'static str {
        "text_case_convert"
    }
    fn description(&self) -> &'static str {
        "Convert text between case conventions. Tokenizes the input by whitespace, punctuation, and case-changes, then re-renders in the target style. Handles existing snake / kebab / camel / pascal input correctly."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CaseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CaseArgs>()?;
            let tokens = tokenize(&a.text);
            let out = match a.target.trim().to_ascii_lowercase().as_str() {
                "snake" => tokens
                    .iter()
                    .map(|t| t.to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join("_"),
                "kebab" => tokens
                    .iter()
                    .map(|t| t.to_ascii_lowercase())
                    .collect::<Vec<_>>()
                    .join("-"),
                "camel" => {
                    let mut out = String::new();
                    for (i, t) in tokens.iter().enumerate() {
                        if i == 0 {
                            out.push_str(&t.to_ascii_lowercase());
                        } else {
                            out.push_str(&capitalize(t));
                        }
                    }
                    out
                }
                "pascal" => tokens.iter().map(|t| capitalize(t)).collect::<String>(),
                "screaming" | "screaming_snake" | "constant" => tokens
                    .iter()
                    .map(|t| t.to_ascii_uppercase())
                    .collect::<Vec<_>>()
                    .join("_"),
                "screaming_kebab" => tokens
                    .iter()
                    .map(|t| t.to_ascii_uppercase())
                    .collect::<Vec<_>>()
                    .join("-"),
                "sentence" => {
                    let joined = tokens
                        .iter()
                        .map(|t| t.to_ascii_lowercase())
                        .collect::<Vec<_>>()
                        .join(" ");
                    capitalize(&joined)
                }
                "title" => tokens
                    .iter()
                    .map(|t| capitalize(t))
                    .collect::<Vec<_>>()
                    .join(" "),
                other => return Err(invalid(format!("unknown target case `{other}`"))),
            };
            Ok(text_result(
                json!({"target": a.target, "result": out, "tokens": tokens}).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Sentence -> snake_case",
                args: r#"{"text": "Hello World!", "target": "snake"}"#,
                note: Some("Returns `hello_world`."),
            },
            SkillExample {
                title: "PascalCase -> kebab",
                args: r#"{"text": "HTTPResponseCode", "target": "kebab"}"#,
                note: Some("Tokenizes `HTTP` + `Response` + `Code` -> `http-response-code`."),
            },
            SkillExample {
                title: "snake -> camelCase",
                args: r#"{"text": "my_variable_name", "target": "camel"}"#,
                note: None,
            },
            SkillExample {
                title: "SCREAMING -> Title Case",
                args: r#"{"text": "OPEN_FILE_HANDLE", "target": "title"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert an identifier between naming conventions without manual splitting.",
            "Tokenize tricky cases (HTTPResponse, IBM2x) correctly.",
        ]
    }
}

fn tokenize(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_upper = false;
    let mut prev_alpha = false;
    for c in s.chars() {
        let is_sep = c.is_whitespace() || matches!(c, '_' | '-' | '.' | '/' | '\\');
        let alpha = c.is_ascii_alphanumeric();
        if is_sep {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_upper = false;
            prev_alpha = false;
            continue;
        }
        if !alpha {
            continue;
        }
        let upper = c.is_ascii_uppercase();
        let digit = c.is_ascii_digit();
        if !cur.is_empty() {
            // Split on alpha-case change but NOT inside acronyms (HTTP -> stays together until lowercase follows).
            if upper && !prev_upper && prev_alpha {
                out.push(std::mem::take(&mut cur));
            } else if prev_upper && !upper && !digit && cur.len() > 1 {
                // HTTPResponse -> HTTP | Response: pop the last char into a new token.
                let last = cur.pop().unwrap();
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                cur.push(last);
            }
        }
        cur.push(c);
        prev_upper = upper;
        prev_alpha = true;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(first) => first.to_ascii_uppercase().to_string() + &c.as_str().to_ascii_lowercase(),
    }
}

// ---------- slugify ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SlugArgs {
    /// Text to slugify.
    text: String,
    /// Separator between slug parts. Defaults to `-`.
    #[serde(default)]
    separator: Option<String>,
}

pub struct TextSlugify;
impl Skill for TextSlugify {
    fn name(&self) -> &'static str {
        "text_slugify"
    }
    fn description(&self) -> &'static str {
        "Produce a URL-safe slug: lowercase ASCII, collapse runs of non-alphanumerics into the separator (default `-`), strip leading/trailing separators. Unicode letters are kept if ASCII-alphanumeric after normalization; otherwise dropped (we don't ship full ICU)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SlugArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SlugArgs>()?;
            let sep = a.separator.as_deref().unwrap_or("-");
            let lower = a.text.to_ascii_lowercase();
            let mut out = String::new();
            let mut in_sep = true;
            for c in lower.chars() {
                if c.is_ascii_alphanumeric() {
                    out.push(c);
                    in_sep = false;
                } else if !in_sep {
                    out.push_str(sep);
                    in_sep = true;
                }
            }
            let trimmed = out
                .trim_matches(sep.chars().next().unwrap_or('-'))
                .to_string();
            Ok(text_result(
                json!({"slug": trimmed, "separator": sep}).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Article title",
                args: r#"{"text": "Hello, World! What's up?"}"#,
                note: Some("Returns `hello-world-what-s-up`."),
            },
            SkillExample {
                title: "Underscore separator",
                args: r#"{"text": "Multi   space line", "separator": "_"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Generate a URL-safe identifier from a title.",
            "Normalize text for filename use.",
        ]
    }
}

// ---------- edit distance ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EditArgs {
    /// First string.
    a: String,
    /// Second string.
    b: String,
}

pub struct TextEditDistance;
impl Skill for TextEditDistance {
    fn name(&self) -> &'static str {
        "text_edit_distance"
    }
    fn description(&self) -> &'static str {
        "Levenshtein edit distance (insertions / deletions / substitutions, each cost 1) between two strings, plus the normalized similarity = 1 - distance / max(len). Works on Unicode code points."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EditArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EditArgs>()?;
            let d = levenshtein(&a.a, &a.b);
            let max_len = a.a.chars().count().max(a.b.chars().count());
            let sim = if max_len == 0 {
                1.0
            } else {
                1.0 - (d as f64 / max_len as f64)
            };
            Ok(text_result(
                json!({"distance": d, "similarity": sim}).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Classic",
                args: r#"{"a": "kitten", "b": "sitting"}"#,
                note: Some("Distance 3 — substitute k→s, e→i, insert g."),
            },
            SkillExample {
                title: "Identical",
                args: r#"{"a": "same", "b": "same"}"#,
                note: Some("Distance 0, similarity 1.0."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute a typo distance between user input and a known dictionary entry.",
            "Quantify similarity between two strings for fuzzy matching.",
        ]
    }
}

fn levenshtein(a: &str, b: &str) -> usize {
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    let n = av.len();
    let m = bv.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur = vec![0usize; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = if av[i - 1] == bv[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

// ---------- line diff ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DiffArgs {
    /// "Before" text.
    a: String,
    /// "After" text.
    b: String,
}

pub struct TextDiff;
impl Skill for TextDiff {
    fn name(&self) -> &'static str {
        "text_diff"
    }
    fn description(&self) -> &'static str {
        "Line-level diff between two texts via the longest-common-subsequence algorithm. Returns added / removed / unchanged line counts and the unified-diff-style hunk list."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DiffArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DiffArgs>()?;
            let al: Vec<&str> = a.a.lines().collect();
            let bl: Vec<&str> = a.b.lines().collect();
            let ops = diff_lines(&al, &bl);
            let mut added = 0usize;
            let mut removed = 0usize;
            let mut unchanged = 0usize;
            let mut hunks: Vec<serde_json::Value> = Vec::new();
            for (op, line) in &ops {
                match op {
                    '+' => {
                        added += 1;
                        hunks.push(json!({"op": "+", "line": line}));
                    }
                    '-' => {
                        removed += 1;
                        hunks.push(json!({"op": "-", "line": line}));
                    }
                    _ => {
                        unchanged += 1;
                    }
                }
            }
            Ok(text_result(
                json!({
                    "added": added,
                    "removed": removed,
                    "unchanged": unchanged,
                    "hunks": hunks,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Insert and replace",
            args: r#"{"a": "line one\nline two\n", "b": "line one\nline two and a half\nline three\n"}"#,
            note: Some("Reports the added lines."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Summarize what changed between two texts without a manual scan.",
            "Count added / removed lines for a code-review summary.",
        ]
    }
}

fn diff_lines<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(char, &'a str)> {
    let n = a.len();
    let m = b.len();
    // LCS table.
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..n {
        for j in 0..m {
            lcs[i + 1][j + 1] = if a[i] == b[j] {
                lcs[i][j] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut ops: Vec<(char, &'a str)> = Vec::new();
    let mut i = n;
    let mut j = m;
    while i > 0 || j > 0 {
        if i > 0 && j > 0 && a[i - 1] == b[j - 1] {
            ops.push(('=', a[i - 1]));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || lcs[i][j - 1] >= lcs[i - 1][j]) {
            ops.push(('+', b[j - 1]));
            j -= 1;
        } else {
            ops.push(('-', a[i - 1]));
            i -= 1;
        }
    }
    ops.reverse();
    ops
}

// ---------- word/char counts ----------
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CountArgs {
    /// Text to measure.
    text: String,
}

pub struct TextWordCount;
impl Skill for TextWordCount {
    fn name(&self) -> &'static str {
        "text_word_count"
    }
    fn description(&self) -> &'static str {
        "Character / word / line / sentence counts plus a rough reading-time estimate (200 WPM). `chars` counts Unicode scalars; `chars_no_whitespace` strips whitespace; `bytes` is the UTF-8 byte length."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CountArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CountArgs>()?;
            let chars = a.text.chars().count();
            let chars_no_ws = a.text.chars().filter(|c| !c.is_whitespace()).count();
            let words = a.text.split_whitespace().count();
            let lines = a.text.lines().count();
            let sentences = a
                .text
                .split(['.', '!', '?'])
                .filter(|s| !s.trim().is_empty())
                .count();
            let read_secs = (words as f64 * 60.0 / 200.0).round() as u64;
            Ok(text_result(
                json!({
                    "chars": chars,
                    "chars_no_whitespace": chars_no_ws,
                    "bytes": a.text.len(),
                    "words": words,
                    "lines": lines,
                    "sentences": sentences,
                    "reading_time_seconds_at_200wpm": read_secs,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Article",
            args: r#"{"text": "Hello, world! How are you today?"}"#,
            note: None,
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute exact word / char counts for a document.",
            "Estimate reading time for a piece of content.",
        ]
    }
}

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "text"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Text transformation + measurement: case conversion (snake/kebab/camel/pascal/screaming/sentence/title), slugify, Levenshtein distance, line diff, word/char/sentence counts. Pure local compute."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some("1. `text_case_convert { text: \"Hello World\", target: \"snake\" }` — normalize identifier.\n2. `text_edit_distance { a: \"recieve\", b: \"receive\" }` — typo check.\n3. `text_diff { a: \"<old>\", b: \"<new>\" }` — what changed.")
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(TextCaseConvert),
        Box::new(TextSlugify),
        Box::new(TextEditDistance),
        Box::new(TextDiff),
        Box::new(TextWordCount),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn levenshtein_kitten_sitting() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }
    #[test]
    fn tokenize_pascal() {
        assert_eq!(
            tokenize("HTTPResponseCode"),
            vec!["HTTP", "Response", "Code"]
        );
    }
    #[test]
    fn tokenize_snake() {
        assert_eq!(tokenize("my_variable_name"), vec!["my", "variable", "name"]);
    }
    #[test]
    fn capitalize_basic() {
        assert_eq!(capitalize("hello"), "Hello");
    }
    #[test]
    fn diff_basic_lines() {
        let a = ["a", "b", "c"];
        let b = ["a", "X", "c"];
        let ops = diff_lines(&a, &b);
        // Should have +X and -b somewhere.
        assert!(ops.iter().any(|(op, l)| *op == '+' && *l == "X"));
        assert!(ops.iter().any(|(op, l)| *op == '-' && *l == "b"));
    }
}
