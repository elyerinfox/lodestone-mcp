//! Retrieval skills — fetch one already-identified resource: a page's readable
//! text (HTTP or headless render), a PDF (read or generate), or a repo file. This
//! module owns the low-level retrieval primitives (raw-file URL resolution,
//! readable-page fetch, PDF text extraction); rendering lives in
//! [`crate::browser`], and the Wayback Machine in [`crate::skills::archive`]
//! (which reuses [`fetch_readable`]).

use std::sync::{Arc, LazyLock};

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use regex::Regex;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::{html_to_text, truncate_chars};
use crate::{browser, internal, invalid, text_result, util};

// ---------------------------------------------------------------------------
// Raw file fetch across forges (no token):
//   GitHub : github.com/.../blob/<ref>/<path>  → raw.githubusercontent.com/.../<ref>/<path>
//   GitLab : <host>/.../-/blob/<ref>/<path>     → <host>/.../-/raw/<ref>/<path>
//   Gitea  : <host>/o/r/src/branch/<ref>/<path> → <host>/o/r/raw/branch/<ref>/<path>
// ---------------------------------------------------------------------------

static GH_BLOB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://github\.com/([^/]+)/([^/]+)/(?:blob|raw)/([^/]+)/(.+)$").unwrap()
});
static GITEA_SRC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/src/(branch|commit|tag)/").unwrap());
static SHORTHAND_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^/\s]+)/([^/\s]+)/(.+)$").unwrap());
static LINE_FRAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#L(\d+)(?:[-C]+L?(\d+))?$").unwrap());

/// Resolved raw download target(s) plus any `#L..` line range from the input.
pub struct RawTarget {
    pub candidates: Vec<String>,
    pub line_range: Option<(usize, usize)>,
}

/// Resolve a GitHub/GitLab/Gitea blob (or raw) URL — or a GitHub
/// `owner/repo/path` shorthand — into raw download target(s).
fn resolve_raw_file(input: &str) -> Result<RawTarget> {
    let input = input.trim();
    let (base, line_range) = split_line_fragment(input);
    let single = |url: String| {
        Ok(RawTarget {
            candidates: vec![url],
            line_range,
        })
    };

    if base.starts_with("https://raw.githubusercontent.com/")
        || base.starts_with("http://raw.githubusercontent.com/")
        || base.contains("/-/raw/")
        || base.contains("/raw/branch/")
        || base.contains("/raw/commit/")
        || base.contains("/raw/tag/")
    {
        return single(base.to_string());
    }

    if let Some(c) = GH_BLOB_RE.captures(base) {
        return single(format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            &c[1], &c[2], &c[3], &c[4]
        ));
    }

    if base.contains("/-/blob/") {
        return single(base.replacen("/-/blob/", "/-/raw/", 1));
    }

    if GITEA_SRC_RE.is_match(base) {
        return single(GITEA_SRC_RE.replace(base, "/raw/$1/").into_owned());
    }

    if !base.contains("://") {
        if let Some(c) = SHORTHAND_RE.captures(base) {
            let (owner, repo, path) = (&c[1], &c[2], &c[3]);
            let candidates = ["main", "master"]
                .iter()
                .map(|r| format!("https://raw.githubusercontent.com/{owner}/{repo}/{r}/{path}"))
                .collect();
            return Ok(RawTarget {
                candidates,
                line_range,
            });
        }
    }

    Err(anyhow!(
        "could not parse '{input}' as a GitHub/GitLab/Gitea file URL or an 'owner/repo/path' reference"
    ))
}

fn split_line_fragment(input: &str) -> (&str, Option<(usize, usize)>) {
    if let Some(c) = LINE_FRAG_RE.captures(input) {
        let whole = c.get(0).unwrap();
        let start: usize = c[1].parse().unwrap_or(0);
        let end: usize = c
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(start);
        if start > 0 {
            return (&input[..whole.start()], Some((start, end.max(start))));
        }
    }
    (input, None)
}

/// GET a URL, returning `(body, status)`.
async fn fetch_text(client: &Client, url: &str) -> Result<(String, reqwest::StatusCode)> {
    let resp = client.get(url).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    Ok((body, status))
}

/// Fetch a URL and return readable text (HTML → text; PDFs text-extracted).
/// `pub(crate)` so the archive skill can read a resolved snapshot.
pub(crate) async fn fetch_readable(client: &Client, url: &str, max_chars: usize) -> Result<String> {
    let resp = client
        .get(url)
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/pdf,*/*",
        )
        .send()
        .await?
        .error_for_status()?;
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = resp.bytes().await?;

    if ctype.contains("pdf")
        || url
            .split('?')
            .next()
            .unwrap_or(url)
            .to_ascii_lowercase()
            .ends_with(".pdf")
        || bytes.starts_with(b"%PDF")
    {
        return extract_pdf_text(bytes.to_vec(), max_chars).await;
    }

    let body = String::from_utf8_lossy(&bytes).into_owned();
    let text = if ctype.contains("html") || body.trim_start().starts_with('<') {
        html_to_text(&body)
    } else {
        body
    };
    Ok(truncate_chars(&text, max_chars))
}

/// Extract a PDF's text layer locally (no external service). Runs the CPU-bound
/// parse off the async runtime. Errors for scanned/no-text-layer PDFs.
async fn extract_pdf_text(bytes: Vec<u8>, max_chars: usize) -> Result<String> {
    let text = tokio::task::spawn_blocking(move || pdf_extract::extract_text_from_mem(&bytes))
        .await
        .map_err(|e| anyhow!("PDF extraction task failed: {e}"))?
        .map_err(|e| anyhow!("could not extract PDF text (scanned or unsupported?): {e}"))?;
    if text.trim().is_empty() {
        return Err(anyhow!(
            "the PDF has no extractable text layer (it may be scanned images)"
        ));
    }
    Ok(truncate_chars(text.trim(), max_chars))
}

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
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let text = fetch_readable(&server.http, &args.url, max)
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
            if let Some(cached) = server.retrieval_get(&key).await {
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
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let bytes: Vec<u8> = if src.starts_with("http://") || src.starts_with("https://") {
                // Shared fetch: local file store → a constellation peer → the source. Lets a
                // PDF cached by one node (arXiv, IETF, …) serve the mesh instead of
                // every node re-hitting the rate-limited source.
                server.fetch_bytes_shared(&src).await.map_err(internal)?
            } else {
                std::fs::read(&src)
                    .map_err(|e| invalid(format!("could not read file '{src}': {e}")))?
            };
            let text = extract_pdf_text(bytes, max).await.map_err(internal)?;
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
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let target = resolve_raw_file(&args.target).map_err(invalid)?;

            let mut last_status = None;
            let mut fetched: Option<(String, String)> = None;
            for url in &target.candidates {
                let (body, status) = fetch_text(&server.http, url).await.map_err(internal)?;
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
        Box::new(WebpageToPdf),
        Box::new(ReadPdf),
        Box::new(FetchRepoFile),
    ]
}

#[cfg(test)]
mod tests {
    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent(crate::LODESTONE_UA)
            .build()
            .unwrap()
    }

    /// example.com is the IANA-blessed stable test domain — won't move.
    #[tokio::test]
    #[ignore]
    async fn fetch_page_live() {
        let r = http()
            .get("https://example.com/")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let body = r.text().await.unwrap();
        assert!(
            body.contains("Example Domain"),
            "example.com schema drift — got: {}",
            &body[..body.len().min(200)]
        );
    }

    /// Raw GitHub: stable URL pattern. Pull the lodestone README as a known
    /// file the fetch_repo_file resolver targets.
    #[tokio::test]
    #[ignore]
    async fn fetch_repo_file_github_raw_live() {
        let r = http()
            .get("https://raw.githubusercontent.com/rust-lang/rust/master/README.md")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let body = r.text().await.unwrap();
        assert!(
            body.contains("Rust"),
            "raw.githubusercontent.com schema drift"
        );
    }
}
