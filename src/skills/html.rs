//! `html_render` — execute HTML/JS in headless Chrome and capture diagnostics.
//!
//! Either give it a URL to navigate to OR a raw HTML string. The page runs
//! for `wait_ms` (default 1500). The tool returns:
//!   * every `console.log/info/warn/error/debug` call (level + concatenated args + source / line);
//!   * every uncaught JS exception (text + stack);
//!   * every network failure (DNS error, blocked, connection refused, …);
//!   * every HTTP 4xx / 5xx response (URL + status);
//!   * final page title, final URL after redirects, total elapsed time.
//!
//! Useful for verifying a generated UI / chart / snippet actually renders
//! cleanly — pipe `chart_interactive`'s HTML through this before sending it
//! on, or load any URL and see what its console / network look like.
//!
//! On by default behind `[html].enabled`. Requires the headless browser
//! (the same Chrome / Chromium used by `render_page` and the search
//! provider rendering — see `[google].chrome_path`).

use std::fmt::Write as _;
use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::browser::{shared_global, PageRenderer, RenderInput};
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

pub const TOOL_NAMES: &[&str] = &["html_render"];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HtmlRenderArgs {
    /// Raw HTML to render. Use this for verifying generated UIs, charts,
    /// `chart_interactive` output, etc. Mutually exclusive with `url`.
    #[serde(default)]
    html: Option<String>,
    /// URL to navigate to. Mutually exclusive with `html`.
    #[serde(default)]
    url: Option<String>,
    /// How long to let JavaScript run before snapshotting, in milliseconds.
    /// Default 1500. Capped at 30000.
    #[serde(default)]
    wait_ms: Option<u64>,
}

pub struct HtmlRender;
impl Skill for HtmlRender {
    fn name(&self) -> &'static str {
        "html_render"
    }
    fn description(&self) -> &'static str {
        "Render an HTML snippet OR a URL in headless Chrome and capture diagnostics: every \
        console call (log / info / warn / error / debug) with source + line, every uncaught JS \
        exception with stack trace, every network failure (DNS / connection / blocked / CORS), \
        every HTTP 4xx / 5xx response, plus final title / URL / elapsed time. Use to verify \
        that a generated UI or `chart_interactive` HTML actually runs cleanly before shipping \
        it. Defaults: `wait_ms=1500`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HtmlRenderArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<HtmlRenderArgs>()?;
            let wait_ms = args.wait_ms.unwrap_or(1500).clamp(0, 30_000);
            let input = match (args.html.as_deref(), args.url.as_deref()) {
                (Some(h), None) if !h.trim().is_empty() => RenderInput::Html(h),
                (None, Some(u)) if !u.trim().is_empty() => RenderInput::Url(u),
                (Some(_), Some(_)) => {
                    return Err(invalid(
                        "supply EITHER `html` OR `url`, not both".to_string(),
                    ))
                }
                _ => return Err(invalid("supply one of `html` or `url`".to_string())),
            };
            let diag = shared_global()
                .render_diagnostics(input, wait_ms)
                .await
                .map_err(|e| internal(anyhow::anyhow!("{e}")))?;
            let out = format_diagnostics(&diag);
            Ok(text_result(out))
        })
    }
}

fn format_diagnostics(d: &crate::browser::PageDiagnostics) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} (load: {} ms)\n  url: {}",
        if d.title.is_empty() {
            "<no title>"
        } else {
            &d.title
        },
        d.elapsed_ms,
        d.final_url,
    );
    // Counts header — quick "did anything go wrong" summary.
    let console_errors = d.console.iter().filter(|c| c.level == "error").count();
    let console_warns = d.console.iter().filter(|c| c.level == "warning").count();
    let _ = writeln!(
        out,
        "  diagnostics: {} console event{} ({} error, {} warning) · {} JS exception{} · {} \
         network failure{} · {} HTTP error{}",
        d.console.len(),
        plural(d.console.len()),
        console_errors,
        console_warns,
        d.exceptions.len(),
        plural(d.exceptions.len()),
        d.network_failures.len(),
        plural(d.network_failures.len()),
        d.http_errors.len(),
        plural(d.http_errors.len()),
    );

    if !d.console.is_empty() {
        out.push_str("\n[console]\n");
        for c in &d.console {
            let loc = match (c.source_url.as_deref(), c.line) {
                (Some(u), Some(l)) => format!("  ({u}:{l})"),
                (Some(u), None) => format!("  ({u})"),
                _ => String::new(),
            };
            let t: String = c.text.chars().take(400).collect();
            let _ = writeln!(out, "  [{:>7}] {}{loc}", c.level, t);
        }
    }
    if !d.exceptions.is_empty() {
        out.push_str("\n[exceptions]\n");
        for ex in &d.exceptions {
            let loc = match (ex.source_url.as_deref(), ex.line, ex.column) {
                (Some(u), Some(l), Some(c)) => format!(" at {u}:{l}:{c}"),
                (Some(u), Some(l), None) => format!(" at {u}:{l}"),
                _ => String::new(),
            };
            let _ = writeln!(out, "  ⚠ {}{loc}", ex.text);
            if let Some(st) = ex.stack.as_deref() {
                out.push_str(st);
                out.push('\n');
            }
        }
    }
    if !d.network_failures.is_empty() {
        out.push_str("\n[network failures]\n");
        for nf in &d.network_failures {
            let _ = writeln!(
                out,
                "  ✗ {} {} — {}",
                nf.resource_type, nf.url, nf.error_text
            );
        }
    }
    if !d.http_errors.is_empty() {
        out.push_str("\n[HTTP errors]\n");
        for h in &d.http_errors {
            let _ = writeln!(out, "  {} {} ({})", h.status, h.url, h.resource_type);
        }
    }
    out
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(HtmlRender)]
}
