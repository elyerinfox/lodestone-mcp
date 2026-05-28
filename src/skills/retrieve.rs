//! Retrieval skills — fetch one already-identified resource: a page's readable
//! text (HTTP or headless render), a PDF (read or generate), a repo file, or a
//! Wayback snapshot. The heavy lifting lives in [`crate::retrieve`] and
//! [`crate::browser`]; these skills are the thin tool layer over them.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{browser, internal, invalid, retrieve, text_result, util};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FetchPageArgs {
    /// Absolute URL of the page to fetch.
    url: String,
    /// Max characters of extracted text to return. Omit for the server default;
    /// capped by the server's `[retrieval].max_chars`. Increase for full pages.
    #[serde(default)]
    max_chars: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RenderPageArgs {
    /// Absolute URL of the page to render.
    url: String,
    /// Max characters of extracted text to return. Omit for the server default;
    /// capped by the server's `[retrieval].max_chars`. Increase for full pages.
    #[serde(default)]
    max_chars: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaybackFetchArgs {
    /// Absolute URL to look up in the Internet Archive Wayback Machine.
    url: String,
    /// Optional snapshot target as `YYYYMMDD` or `YYYYMMDDhhmmss`; the closest
    /// capture is returned. Omit for the most recent snapshot.
    #[serde(default)]
    timestamp: Option<String>,
    /// Max characters of extracted text to return. Omit for the server default;
    /// capped by the server's `[retrieval].max_chars`. Increase for full pages.
    #[serde(default)]
    max_chars: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FetchFileArgs {
    /// A repo file URL — GitHub (`/blob/`), GitLab (`/-/blob/`), or Gitea/
    /// Codeberg (`/src/branch/`) — a raw URL, or a GitHub `owner/repo/path/to/file`
    /// shorthand. A trailing `#L10-L40` line range is honored if present.
    target: String,
    /// First line to return (1-based, inclusive). Optional.
    #[serde(default)]
    start_line: Option<usize>,
    /// Last line to return (1-based, inclusive). Optional.
    #[serde(default)]
    end_line: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WebpageToPdfArgs {
    /// Absolute URL of the page to render to PDF (via the local headless browser).
    url: String,
    /// Output file path. Omit to write to a temp file; the saved path is returned.
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReadPdfArgs {
    /// A PDF to read: an absolute URL or a local file path.
    source: String,
    /// Max characters of extracted text to return. Omit for the server default.
    #[serde(default)]
    max_chars: Option<u32>,
}

fn slice_lines(s: &str, start: usize, end: usize) -> String {
    let start = start.max(1);
    let lines: Vec<&str> = s.lines().collect();
    let total = lines.len();
    let end = end.min(total);
    if start > total {
        return format!("(file has only {total} lines; requested start {start})");
    }
    let width = end.to_string().len();
    lines[start - 1..end]
        .iter()
        .enumerate()
        .map(|(i, l)| format!("{:>width$} | {l}", start + i, width = width))
        .collect::<Vec<_>>()
        .join("\n")
}

pub struct FetchPage;
impl Skill for FetchPage {
    fn name(&self) -> &'static str {
        "fetch_page"
    }
    fn description(&self) -> &'static str {
        "Fetch a web page over plain HTTP and return its readable text (HTML stripped). The default \
        way to read a page (docs, blogs, articles). Output is truncated to a character budget — if \
        the text ends with a '[... truncated ...]' marker and you need more, call again with a \
        larger `max_chars`. If it fails or comes back empty (JS-heavy/SPA), try `render_page`; for a \
        page that's down/changed/blocked, try `wayback_fetch`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FetchPageArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<FetchPageArgs>()?;
            let max = server.clamp_chars(args.max_chars);
            let key = format!("page|{max}|{}", args.url);
            if let Some(cached) = server.retrieval_get(&key) {
                return Ok(text_result(cached));
            }
            let text = retrieve::fetch_readable(&server.http, &args.url, max)
                .await
                .map_err(internal)?;
            let out = format!("Source: {}\n\n{}", args.url, text);
            if !text.is_empty() {
                server.retrieval_put(key, &out);
            }
            Ok(text_result(out))
        })
    }
}

pub struct RenderPage;
impl Skill for RenderPage {
    fn name(&self) -> &'static str {
        "render_page"
    }
    fn description(&self) -> &'static str {
        "Fetch a web page through a real headless browser (executes JavaScript) and return its \
        readable text. Use for JS-heavy/SPA pages, or when `fetch_page` is empty or blocked. Output \
        is truncated to a character budget — pass a larger `max_chars` if the text is cut off. \
        Slower than fetch_page and needs a local Chrome/Chromium at runtime."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RenderPageArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use crate::browser::PageRenderer;
            let (server, args) = ctx.parse::<RenderPageArgs>()?;
            let max = server.clamp_chars(args.max_chars);
            let key = format!("render|{max}|{}", args.url);
            if let Some(cached) = server.retrieval_get(&key) {
                return Ok(text_result(cached));
            }
            let html = browser::shared_global()
                .render(&args.url)
                .await
                .map_err(internal)?;
            let text = util::truncate_chars(&util::html_to_text(&html), max);
            let out = format!("Source (rendered): {}\n\n{}", args.url, text);
            if !text.is_empty() {
                server.retrieval_put(key, &out);
            }
            Ok(text_result(out))
        })
    }
}

pub struct WaybackFetch;
impl Skill for WaybackFetch {
    fn name(&self) -> &'static str {
        "wayback_fetch"
    }
    fn description(&self) -> &'static str {
        "Fetch a page from the Internet Archive Wayback Machine (keyless). Returns the readable \
        text of the closest archived snapshot. Useful when a page is down, paywalled, changed, or \
        blocking automated access, or to view a historical version. Output is truncated to a \
        character budget — pass a larger `max_chars` to get more."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WaybackFetchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<WaybackFetchArgs>()?;
            let max = server.clamp_chars(args.max_chars);
            let key = format!(
                "wayback|{max}|{}|{}",
                args.timestamp.as_deref().unwrap_or(""),
                args.url
            );
            if let Some(cached) = server.retrieval_get(&key) {
                return Ok(text_result(cached));
            }
            let (snapshot, text) =
                retrieve::wayback_fetch(&server.http, &args.url, args.timestamp.as_deref(), max)
                    .await
                    .map_err(internal)?;
            let out = format!("Source (archived): {snapshot}\n\n{text}");
            if !text.is_empty() {
                server.retrieval_put(key, &out);
            }
            Ok(text_result(out))
        })
    }
}

pub struct WebpageToPdf;
impl Skill for WebpageToPdf {
    fn name(&self) -> &'static str {
        "webpage_to_pdf"
    }
    fn description(&self) -> &'static str {
        "Render a web page to a PDF file locally via the headless browser (no external service). \
        Saves to `path`, or a temp file if omitted, and returns the saved path. Needs a local \
        Chrome/Chromium at runtime."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WebpageToPdfArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use crate::browser::PageRenderer;
            let (_server, args) = ctx.parse::<WebpageToPdfArgs>()?;
            let bytes = browser::shared_global()
                .render_pdf(&args.url)
                .await
                .map_err(internal)?;
            let path = match args
                .path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(p) => std::path::PathBuf::from(p),
                None => {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    args.url.hash(&mut h);
                    std::env::temp_dir().join(format!("lodestone-{:x}.pdf", h.finish()))
                }
            };
            std::fs::write(&path, &bytes).map_err(|e| {
                internal(anyhow::anyhow!("could not write '{}': {e}", path.display()))
            })?;
            Ok(text_result(format!(
                "Saved {} ({} bytes) from {}",
                path.display(),
                bytes.len(),
                args.url
            )))
        })
    }
}

pub struct ReadPdf;
impl Skill for ReadPdf {
    fn name(&self) -> &'static str {
        "read_pdf"
    }
    fn description(&self) -> &'static str {
        "Read a PDF and return its text, extracted locally (no external service). `source` is an \
        absolute URL or a local file path. Scanned/image-only PDFs (no text layer) return an error \
        rather than text."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ReadPdfArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ReadPdfArgs>()?;
            let max = server.clamp_chars(args.max_chars);
            let src = args.source.trim().to_string();
            let key = format!("readpdf|{max}|{src}");
            if let Some(cached) = server.retrieval_get(&key) {
                return Ok(text_result(cached));
            }
            let bytes: Vec<u8> = if src.starts_with("http://") || src.starts_with("https://") {
                server
                    .http
                    .get(&src)
                    .send()
                    .await
                    .map_err(|e| internal(e.into()))?
                    .error_for_status()
                    .map_err(|e| internal(e.into()))?
                    .bytes()
                    .await
                    .map_err(|e| internal(e.into()))?
                    .to_vec()
            } else {
                std::fs::read(&src)
                    .map_err(|e| invalid(format!("could not read file '{src}': {e}")))?
            };
            let text = retrieve::extract_pdf_text(bytes, max)
                .await
                .map_err(internal)?;
            let out = format!("PDF: {src}\n\n{text}");
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct FetchRepoFile;
impl Skill for FetchRepoFile {
    fn name(&self) -> &'static str {
        "fetch_repo_file"
    }
    fn description(&self) -> &'static str {
        "Retrieve the full contents of a repository file (no token) from GitHub, GitLab, or \
        Gitea/Codeberg. Accepts a blob URL, a raw URL, or a GitHub `owner/repo/path` shorthand. \
        Optionally restrict to a line range."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FetchFileArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<FetchFileArgs>()?;
            let key = format!(
                "file|{}|{}|{}",
                args.target,
                args.start_line.unwrap_or(0),
                args.end_line.unwrap_or(0)
            );
            if let Some(cached) = server.retrieval_get(&key) {
                return Ok(text_result(cached));
            }
            let target = retrieve::resolve_raw_file(&args.target).map_err(invalid)?;

            let mut last_status = None;
            let mut fetched: Option<(String, String)> = None;
            for url in &target.candidates {
                let (body, status) = retrieve::fetch_text(&server.http, url)
                    .await
                    .map_err(internal)?;
                if status.is_success() {
                    fetched = Some((url.clone(), body));
                    break;
                }
                last_status = Some(status);
            }

            let (url, body) = match fetched {
                Some(v) => v,
                None => {
                    return Ok(text_result(format!(
                        "Could not fetch '{}'. Last HTTP status: {}",
                        args.target,
                        last_status
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| "unknown".into())
                    )));
                }
            };

            let range = match (args.start_line, args.end_line) {
                (Some(s), e) => Some((s.max(1), e.unwrap_or(usize::MAX))),
                (None, Some(e)) => Some((1, e)),
                (None, None) => target.line_range,
            };

            let content = match range {
                Some((start, end)) => slice_lines(&body, start, end),
                None => body,
            };

            let out = format!("File: {url}\n\n{content}");
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(FetchPage),
        Box::new(RenderPage),
        Box::new(WaybackFetch),
        Box::new(WebpageToPdf),
        Box::new(ReadPdf),
        Box::new(FetchRepoFile),
    ]
}
