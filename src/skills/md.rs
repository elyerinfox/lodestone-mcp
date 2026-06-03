//! Markdown skills (local compute): Markdown → HTML, HTML → Markdown,
//! and a CommonMark linter. Pure-Rust via `pulldown-cmark` + the existing
//! `html2text` dep for the inverse direction. LLMs often produce malformed
//! Markdown when asked to round-trip HTML; deterministic conversion is the
//! right answer.
//!
//! ## Sources
//!
//! - CommonMark 0.31.2 specification (the grammar `pulldown-cmark` parses).
//! - GitHub-flavored Markdown extensions (tables, strikethrough, task lists,
//!   footnotes) per GFM 0.29-gfm.

use std::sync::Arc;

use futures::future::BoxFuture;
use pulldown_cmark::{html, Event, Options, Parser, Tag};
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::text_result;

fn enabled_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
}

// ---------------------------------------------------------------------------
// md_to_html
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ToHtmlArgs {
    /// Markdown source to render.
    markdown: String,
}

pub struct MdToHtml;
impl Skill for MdToHtml {
    fn name(&self) -> &'static str {
        "md_to_html"
    }
    fn description(&self) -> &'static str {
        "Render CommonMark / GFM Markdown to HTML. Enables tables, footnotes, strikethrough, \
         task lists, and smart punctuation. Pure local compute via pulldown-cmark; no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ToHtmlArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ToHtmlArgs>()?;
            let parser = Parser::new_ext(&args.markdown, enabled_options());
            let mut out = String::with_capacity(args.markdown.len());
            html::push_html(&mut out, parser);
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Heading + bold + link",
                args: r##"{"markdown": "# Title\n\nSee **here**: [docs](https://example.com)."}"##,
                note: Some("Returns the rendered HTML body."),
            },
            SkillExample {
                title: "Fenced code block",
                args: r#"{"markdown": "```rust\nfn main() {}\n```"}"#,
                note: Some("Wraps in <pre><code class=\"language-rust\">."),
            },
            SkillExample {
                title: "Table (GFM extension)",
                args: r#"{"markdown": "| a | b |\n|---|---|\n| 1 | 2 |"}"#,
                note: Some("GFM tables are enabled."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert Markdown that an LLM produced into renderable HTML for display.",
            "Verify what a Markdown source actually renders as before publishing.",
            "Round-trip a document through HTML for downstream processing.",
        ]
    }
}

// ---------------------------------------------------------------------------
// html_to_md
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ToMdArgs {
    /// HTML source.
    html: String,
    /// Line-wrap width in characters. Default 80.
    #[serde(default)]
    width: Option<usize>,
}

pub struct HtmlToMd;
impl Skill for HtmlToMd {
    fn name(&self) -> &'static str {
        "html_to_md"
    }
    fn description(&self) -> &'static str {
        "Convert HTML into Markdown-ish plain text. Backed by html2text — preserves headings, \
         lists, links (as `[text](href)`), code blocks, and tables; strips inline styles and \
         scripts. The output is good enough to round-trip through `md_to_html` for canonical \
         Markdown. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ToMdArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ToMdArgs>()?;
            let width = args.width.unwrap_or(80).max(20);
            let out = html2text::from_read(args.html.as_bytes(), width);
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Article body",
                args: r#"{"html": "<h1>Hello</h1><p>World <a href=\"https://x.com\">link</a>.</p>"}"#,
                note: Some("Returns a heading + paragraph with the link inlined."),
            },
            SkillExample {
                title: "Narrower wrap",
                args: r#"{"html": "<p>Long paragraph of text here.</p>", "width": 40}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Reduce an HTML snippet to readable Markdown-ish text for the LLM to summarize.",
            "Strip inline scripts / styles / tracking pixels while keeping structure.",
            "Round-trip a document via Markdown for re-rendering or storage.",
        ]
    }
}

// ---------------------------------------------------------------------------
// md_lint
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LintArgs {
    /// Markdown source to lint.
    markdown: String,
}

pub struct MdLint;
impl Skill for MdLint {
    fn name(&self) -> &'static str {
        "md_lint"
    }
    fn description(&self) -> &'static str {
        "Lint Markdown for structural issues a CommonMark / GFM parser would silently accept but \
         a reader probably wouldn't intend: unclosed code fences, broken link references, \
         heading levels skipping (h1 → h3 with no h2), and trailing whitespace inside list items. \
         Returns a list of findings with line numbers when possible. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<LintArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<LintArgs>()?;
            let findings = lint_markdown(&args.markdown);
            Ok(text_result(
                json!({
                    "issue_count": findings.len(),
                    "issues": findings,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Heading skip detection",
                args: r##"{"markdown": "# H1\n### H3 (skipped H2)\n"}"##,
                note: Some("Flags the H1 → H3 jump."),
            },
            SkillExample {
                title: "Unclosed code fence",
                args: r#"{"markdown": "```rust\nfn main() {}\n"}"#,
                note: Some("Flags the missing closing fence."),
            },
            SkillExample {
                title: "Clean Markdown",
                args: r##"{"markdown": "# Title\n\nPlain paragraph.\n"}"##,
                note: Some("Returns issue_count=0."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Audit LLM-generated Markdown for structural issues before publishing.",
            "Catch unclosed code fences that silently swallow following content.",
            "Spot heading-hierarchy gaps that hurt navigation / accessibility.",
        ]
    }
}

fn lint_markdown(src: &str) -> Vec<serde_json::Value> {
    let mut findings: Vec<serde_json::Value> = Vec::new();
    let parser = Parser::new_ext(src, enabled_options());
    let mut prev_heading_level: Option<u32> = None;
    for event in parser {
        if let Event::Start(Tag::Heading { level, .. }) = event {
            let level_u32 = level as u32;
            if let Some(prev) = prev_heading_level {
                if level_u32 > prev + 1 {
                    findings.push(json!({
                        "rule": "heading_skip",
                        "severity": "warning",
                        "message": format!("heading jumped from h{prev} to h{level_u32}; consider adding an intermediate level"),
                    }));
                }
            }
            prev_heading_level = Some(level_u32);
        }
    }

    // Catch unclosed fenced code blocks by counting ``` markers.
    let fence_count = src
        .lines()
        .filter(|l| l.trim_start().starts_with("```"))
        .count();
    if fence_count % 2 != 0 {
        findings.push(json!({
            "rule": "unclosed_fence",
            "severity": "error",
            "message": "odd number of ``` fence markers — at least one code block is not closed",
        }));
    }

    // Trailing whitespace on non-empty lines (excluding the "two trailing spaces = hard break" idiom).
    let mut trailing_ws_lines: Vec<usize> = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let trimmed = line.trim_end_matches('\r');
        let trailing = trimmed.len() - trimmed.trim_end().len();
        if trailing > 2 && !trimmed.trim().is_empty() {
            trailing_ws_lines.push(i + 1);
        }
    }
    if !trailing_ws_lines.is_empty() {
        findings.push(json!({
            "rule": "trailing_whitespace",
            "severity": "low",
            "message": format!("trailing whitespace on lines: {}", &trailing_ws_lines.iter().take(20).map(|n| n.to_string()).collect::<Vec<_>>().join(", ")),
            "lines": trailing_ws_lines,
        }));
    }

    // Broken link references: any `[text][ref]` whose `ref` is not later defined as `[ref]: url`.
    let parser = Parser::new_ext(src, enabled_options());
    let mut refs_seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for event in parser {
        if let Event::Start(Tag::Link {
            link_type: pulldown_cmark::LinkType::ReferenceUnknown,
            dest_url,
            ..
        }) = event
        {
            refs_seen.insert(dest_url.to_string());
        }
    }
    for r in refs_seen {
        findings.push(json!({
            "rule": "broken_reference",
            "severity": "warning",
            "message": format!("link reference `[{r}]` has no matching `[{r}]: url` definition"),
        }));
    }

    findings
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "md"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Markdown ↔ HTML conversion plus a CommonMark / GFM linter. Pure local compute via \
         pulldown-cmark + html2text. Tables / footnotes / strikethrough / task lists / smart \
         punctuation extensions enabled."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `md_lint { markdown: \"<your markdown>\" }` — any structural issues?\n\
             2. `md_to_html { markdown: \"<your markdown>\" }` — what does it render as?\n\
             3. `html_to_md { html: \"<rendered html>\" }` — round-trip back to confirm idempotence.",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(MdToHtml), Box::new(HtmlToMd), Box::new(MdLint)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md_to_html_heading() {
        let parser = Parser::new_ext("# Hello", enabled_options());
        let mut out = String::new();
        html::push_html(&mut out, parser);
        assert!(out.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn lint_flags_heading_skip() {
        let f = lint_markdown("# H1\n### H3 (skipped)\n");
        assert!(f.iter().any(|v| v["rule"] == "heading_skip"));
    }

    #[test]
    fn lint_flags_unclosed_fence() {
        let f = lint_markdown("```rust\nfn main() {}\n");
        assert!(f.iter().any(|v| v["rule"] == "unclosed_fence"));
    }

    #[test]
    fn lint_clean_markdown_no_findings() {
        let f = lint_markdown("# Title\n\nParagraph.\n");
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn html_to_md_extracts_text() {
        let out = html2text::from_read("<h1>Hello</h1><p>World.</p>".as_bytes(), 80);
        assert!(out.contains("Hello"));
        assert!(out.contains("World"));
    }
}
