//! lodestone-mcp — an MCP server that searches and retrieves code from the web
//! by scraping search engines and public endpoints. No API keys / tokens.
//!
//! Sources are pluggable: each one implements the [`provider::SearchProvider`]
//! trait and is selected/ordered via configuration (see [`config`]). Retrieval
//! of a specific resource lives in [`retrieve`].
//!
//! Transport: Streamable HTTP, mounted at `/mcp` (works with LM Studio's
//! `url`-style mcp.json entries and any Streamable-HTTP MCP client).

mod browser;
mod config;
mod provider;
mod providers;
mod retrieve;
mod util;

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use rmcp::{
    handler::server::{
        router::tool::{ToolRoute, ToolRouter},
        tool::{parse_json_object, schema_for_type, ToolCallContext},
        wrapper::Parameters,
    },
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData as McpError, ServerHandler,
};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use config::Config;
use provider::{ProviderKind, Registry, SearchQuery, SearchResult};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36";

// ---------------------------------------------------------------------------
// Tool argument schemas
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WebSearchArgs {
    /// The search query. Search-engine operators work (e.g. quotes, `site:`).
    query: String,
    /// Maximum number of results to return. Default 8, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
    /// Fetch results through a real headless browser (executes JS, can bypass
    /// bot-walls/rate-limits) instead of plain HTTP. Slower; needs a local
    /// Chrome/Chromium at runtime.
    #[serde(default)]
    render: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CodeSearchArgs {
    /// What to look for in source code (symbol, function name, snippet, etc.).
    query: String,
    /// Optional language hint to narrow results (e.g. "rust", "python").
    #[serde(default)]
    language: Option<String>,
    /// Maximum number of results to return. Default 10, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
    /// Fetch results through a real headless browser (executes JS, can bypass
    /// bot-walls/rate-limits) instead of plain HTTP. Slower; needs a local
    /// Chrome/Chromium at runtime.
    #[serde(default)]
    render: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FetchPageArgs {
    /// Absolute URL of the page to fetch.
    url: String,
    /// Max characters of extracted text to return. Default 8000, capped 40000.
    #[serde(default)]
    max_chars: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RenderPageArgs {
    /// Absolute URL of the page to render.
    url: String,
    /// Max characters of extracted text to return. Default 8000, capped 40000.
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
    /// Max characters of extracted text to return. Default 8000, capped 40000.
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
struct StackSearchArgs {
    /// The question/problem to search for.
    query: String,
    /// StackExchange site to search. Defaults to the configured site
    /// (e.g. "serverfault", "superuser", "askubuntu", "unix").
    #[serde(default)]
    site: Option<String>,
    /// Maximum number of results to return. Default 8, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
    /// Scrape stackoverflow.com via a headless browser instead of the API
    /// (avoids the API quota; stackoverflow site only). Needs a local
    /// Chrome/Chromium at runtime.
    #[serde(default)]
    render: Option<bool>,
}

/// Arguments for the granular, one-tool-per-provider skills.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ProviderSearchArgs {
    /// The search query.
    query: String,
    /// Maximum number of results to return. Default 10, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
    /// Optional language hint (code providers).
    #[serde(default)]
    language: Option<String>,
    /// Optional StackExchange site slug (qa providers).
    #[serde(default)]
    site: Option<String>,
    /// Fetch via a real headless browser instead of plain HTTP. Slower; needs a
    /// local Chrome/Chromium at runtime.
    #[serde(default)]
    render: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StackAnswersArgs {
    /// A StackExchange question URL or numeric question id.
    question: String,
    /// StackExchange site. Defaults to the configured site. Must match the question's site.
    #[serde(default)]
    site: Option<String>,
    /// Maximum number of answers to return (sorted by votes). Default 3, cap 10.
    #[serde(default)]
    max_answers: Option<u32>,
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Lodestone {
    http: reqwest::Client,
    registry: Arc<Registry>,
    default_se_site: Arc<str>,
    se_key: Arc<str>,
    se_allowed: Arc<[String]>,
    // The filtered tool router; `#[tool_handler(router = self.tool_router)]`
    // uses it for both tool listing and dispatch.
    tool_router: ToolRouter<Lodestone>,
}

#[tool_router]
impl Lodestone {
    fn new(
        registry: Arc<Registry>,
        default_se_site: String,
        se_key: String,
        se_allowed: Vec<String>,
        tools_enabled: &[String],
        tools_disabled: &[String],
    ) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(25))
            .build()
            .expect("failed to build HTTP client");
        let tool_router = build_tool_router(&registry, tools_enabled, tools_disabled);
        Self {
            http,
            registry,
            default_se_site: default_se_site.into(),
            se_key: se_key.into(),
            se_allowed: se_allowed.into(),
            tool_router,
        }
    }

    /// Guardrail: is `site` permitted by the configured StackExchange allowlist?
    fn se_site_allowed(&self, site: &str) -> bool {
        self.se_allowed.is_empty() || self.se_allowed.iter().any(|s| s == site)
    }

    #[tool(
        description = "Search the web (scraped via the configured web providers, no API key). \
        Returns a ranked list of title / URL / snippet. Use `fetch_page` to read a result. Set \
        render=true to fetch via a real headless browser (slower, but can bypass rate-limits/bot-walls)."
    )]
    async fn web_search(
        &self,
        Parameters(args): Parameters<WebSearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let q = SearchQuery {
            text: args.query.clone(),
            language: None,
            site: None,
            limit: clamp(args.max_results, 8, 25),
            render: args.render.unwrap_or(false),
        };
        let (hits, engine) = self
            .registry
            .search(ProviderKind::Web, &self.http, &q)
            .await;
        if hits.is_empty() {
            return Ok(text_result(format!("No web results for: {}", args.query)));
        }
        Ok(text_result(format_web(&args.query, &engine, &hits)))
    }

    #[tool(
        description = "Search source code across public repositories (via the configured code \
        providers, e.g. grep.app then a GitHub-scoped web search). Returns repo, file path and a \
        snippet. Use `fetch_repo_file` on a result URL to read the full file. Set render=true to \
        fetch via a real headless browser (slower, but can bypass rate-limits/bot-walls)."
    )]
    async fn code_search(
        &self,
        Parameters(args): Parameters<CodeSearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let q = SearchQuery {
            text: args.query.clone(),
            language: args.language.clone(),
            site: None,
            limit: clamp(args.max_results, 10, 25),
            render: args.render.unwrap_or(false),
        };
        let (hits, engine) = self
            .registry
            .search(ProviderKind::Code, &self.http, &q)
            .await;
        if hits.is_empty() {
            return Ok(text_result(format!("No code results for: {}", args.query)));
        }
        Ok(text_result(format_code(&args.query, &engine, &hits)))
    }

    #[tool(
        description = "Fetch a web page over plain HTTP and return its readable text (HTML \
        stripped). The default way to read a page (docs, blogs, articles). If it fails or comes \
        back empty (JS-heavy/SPA), try `render_page`; for a page that's down/changed/blocked, try \
        `wayback_fetch`."
    )]
    async fn fetch_page(
        &self,
        Parameters(args): Parameters<FetchPageArgs>,
    ) -> Result<CallToolResult, McpError> {
        let max = clamp(args.max_chars, 8000, 40000);
        let text = retrieve::fetch_readable(&self.http, &args.url, max)
            .await
            .map_err(internal)?;
        Ok(text_result(format!("Source: {}\n\n{}", args.url, text)))
    }

    #[tool(
        description = "Fetch a web page through a real headless browser (executes JavaScript) and \
        return its readable text. Use for JS-heavy/SPA pages, or when `fetch_page` is empty or \
        blocked. Slower than fetch_page and needs a local Chrome/Chromium at runtime."
    )]
    async fn render_page(
        &self,
        Parameters(args): Parameters<RenderPageArgs>,
    ) -> Result<CallToolResult, McpError> {
        use crate::browser::PageRenderer;
        let max = clamp(args.max_chars, 8000, 40000);
        let html = browser::shared_global()
            .render(&args.url)
            .await
            .map_err(internal)?;
        let text = util::truncate_chars(&util::html_to_text(&html), max);
        Ok(text_result(format!(
            "Source (rendered): {}\n\n{}",
            args.url, text
        )))
    }

    #[tool(
        description = "Fetch a page from the Internet Archive Wayback Machine (keyless). Returns \
        the readable text of the closest archived snapshot. Useful when a page is down, paywalled, \
        changed, or blocking automated access, or to view a historical version."
    )]
    async fn wayback_fetch(
        &self,
        Parameters(args): Parameters<WaybackFetchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let max = clamp(args.max_chars, 8000, 40000);
        let (snapshot, text) =
            retrieve::wayback_fetch(&self.http, &args.url, args.timestamp.as_deref(), max)
                .await
                .map_err(internal)?;
        Ok(text_result(format!(
            "Source (archived): {snapshot}\n\n{text}"
        )))
    }

    #[tool(
        description = "Retrieve the full contents of a repository file (no token) from GitHub, \
        GitLab, or Gitea/Codeberg. Accepts a blob URL, a raw URL, or a GitHub `owner/repo/path` \
        shorthand. Optionally restrict to a line range."
    )]
    async fn fetch_repo_file(
        &self,
        Parameters(args): Parameters<FetchFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        let target = retrieve::resolve_raw_file(&args.target).map_err(invalid)?;

        let mut last_status = None;
        let mut fetched: Option<(String, String)> = None; // (url, body)
        for url in &target.candidates {
            let (body, status) = retrieve::fetch_text(&self.http, url)
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

        Ok(text_result(format!("File: {url}\n\n{content}")))
    }

    #[tool(
        description = "Search the Q&A providers (StackOverflow / StackExchange). Returns matching \
        questions with score, answer count and links. Uses the keyless API by default; set \
        render=true to scrape stackoverflow.com via a headless browser (no API quota). Use \
        `stackexchange_answers` to read the actual answers."
    )]
    async fn stackexchange_search(
        &self,
        Parameters(args): Parameters<StackSearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let site = args
            .site
            .clone()
            .unwrap_or_else(|| self.default_se_site.to_string());
        if !self.se_site_allowed(&site) {
            return Err(invalid(format!(
                "site '{site}' is not in the configured StackExchange allowlist"
            )));
        }
        let q = SearchQuery {
            text: args.query.clone(),
            language: None,
            site: Some(site.clone()),
            limit: clamp(args.max_results, 8, 25),
            render: args.render.unwrap_or(false),
        };
        let (hits, _engine) = self.registry.search(ProviderKind::Qa, &self.http, &q).await;
        if hits.is_empty() {
            return Ok(text_result(format!(
                "No {site} results for: {}",
                args.query
            )));
        }
        Ok(text_result(format_qa(&args.query, &site, &hits)))
    }

    #[tool(
        description = "Read a StackExchange question body and its top answers (by votes), \
        including any code blocks. Accepts a question URL or numeric id."
    )]
    async fn stackexchange_answers(
        &self,
        Parameters(args): Parameters<StackAnswersArgs>,
    ) -> Result<CallToolResult, McpError> {
        let site = args
            .site
            .clone()
            .unwrap_or_else(|| self.default_se_site.to_string());
        if !self.se_site_allowed(&site) {
            return Err(invalid(format!(
                "site '{site}' is not in the configured StackExchange allowlist"
            )));
        }
        let max = clamp(args.max_answers, 3, 10);
        let qid = retrieve::extract_question_id(&args.question).ok_or_else(|| {
            invalid(format!(
                "could not find a question id in '{}'",
                args.question
            ))
        })?;

        let (q, a) = retrieve::se_answers(&self.http, &qid, &site, max, &self.se_key)
            .await
            .map_err(internal)?;

        let mut out = String::new();
        if let Some(item) = q
            .get("items")
            .and_then(|i| i.as_array())
            .and_then(|a| a.first())
        {
            let title =
                util::decode_entities(item.get("title").and_then(|x| x.as_str()).unwrap_or(""));
            let link = item.get("link").and_then(|x| x.as_str()).unwrap_or("");
            let body = item.get("body").and_then(|x| x.as_str()).unwrap_or("");
            out.push_str(&format!("QUESTION: {title}\n{link}\n\n"));
            out.push_str(&util::html_to_text(body));
            out.push_str("\n\n");
        } else {
            return Ok(text_result(format!("Question {qid} not found on {site}.")));
        }

        match a.get("items").and_then(|i| i.as_array()) {
            Some(list) if !list.is_empty() => {
                out.push_str(&format!("===== {} ANSWER(S) =====\n", list.len()));
                for (i, ans) in list.iter().enumerate() {
                    let score = ans.get("score").and_then(|x| x.as_i64()).unwrap_or(0);
                    let accepted = ans
                        .get("is_accepted")
                        .and_then(|x| x.as_bool())
                        .unwrap_or(false);
                    let body = ans.get("body").and_then(|x| x.as_str()).unwrap_or("");
                    out.push_str(&format!(
                        "\n----- Answer {} (score {score}{}) -----\n",
                        i + 1,
                        if accepted { ", accepted ✓" } else { "" }
                    ));
                    out.push_str(&util::html_to_text(body));
                    out.push('\n');
                }
            }
            _ => out.push_str("(no answers)"),
        }

        Ok(text_result(util::truncate_chars(&out, 40000)))
    }

    #[tool(
        description = "List the configured search providers and the order they are tried, for \
        web, code and Q&A. Useful to check which sources are active."
    )]
    async fn list_providers(&self) -> Result<CallToolResult, McpError> {
        Ok(text_result(self.registry.describe()))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Lodestone {
    fn get_info(&self) -> ServerInfo {
        let mut implementation = Implementation::from_build_env();
        implementation.name = "lodestone-mcp".to_string();
        implementation.version = env!("CARGO_PKG_VERSION").to_string();
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(implementation)
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "Lodestone scrapes the open web (no API keys) to help you search for and retrieve \
                code and documentation.\n\nTools:\n\
                - web_search: general web search.\n\
                - code_search: search source code in public repositories.\n\
                - fetch_repo_file: download a full file from GitHub/GitLab/Gitea by URL or owner/repo/path.\n\
                - fetch_page: get readable text of any URL over plain HTTP.\n\
                - render_page: get readable text of a URL via a headless browser (JS).\n\
                - wayback_fetch: read a page's archived snapshot from the Wayback Machine.\n\
                - stackexchange_search: find StackOverflow/StackExchange questions.\n\
                - stackexchange_answers: read a question's top answers (with code).\n\
                - list_providers: show which sources are active.\n\
                Each configured provider also has a direct tool named <kind>_<id> \
                (e.g. web_mojeek, code_github, qa_stackoverflow) to target one source.\n\n\
                Typical flow: search (web_search/code_search/stackexchange_search) → then retrieve \
                (fetch_repo_file / fetch_page / render_page / stackexchange_answers) on the best hit."
                    .to_string(),
            )
    }
}

// ---------------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------------

fn format_web(query: &str, engine: &str, hits: &[SearchResult]) -> String {
    let mut out = format!("Web results for \"{query}\" (via {engine}):\n");
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n   {}\n", i + 1, h.title, h.url));
        if !h.snippet.is_empty() {
            out.push_str(&format!("   {}\n", h.snippet));
        }
        if let Some(meta) = &h.meta {
            out.push_str(&format!("   [{meta}]\n"));
        }
    }
    out
}

fn format_code(query: &str, engine: &str, hits: &[SearchResult]) -> String {
    let mut out = format!("Code results for \"{query}\" (via {engine}):\n");
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n", i + 1, h.title));
        if !h.url.is_empty() {
            out.push_str(&format!("   {}\n", h.url));
        }
        if !h.snippet.is_empty() {
            out.push_str(&indent(&h.snippet, "   "));
            out.push('\n');
        }
        if let Some(meta) = &h.meta {
            out.push_str(&format!("   [{meta}]\n"));
        }
    }
    out
}

fn format_qa(query: &str, site: &str, hits: &[SearchResult]) -> String {
    let mut out = format!("{site} results for \"{query}\":\n");
    for (i, h) in hits.iter().enumerate() {
        let score = h.score.unwrap_or(0);
        out.push_str(&format!("\n{}. {}\n", i + 1, h.title));
        out.push_str(&format!("   score {score}"));
        if let Some(meta) = &h.meta {
            out.push_str(&format!(" · {meta}"));
        }
        out.push('\n');
        if !h.url.is_empty() {
            out.push_str(&format!("   {}\n", h.url));
        }
    }
    out.push_str("\nTip: pass a question URL to stackexchange_answers to read answers.");
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the tool router exposing only the configured subset of tools (skills).
/// `enabled` empty = expose all; `disabled` is applied afterward.
fn build_tool_router(
    registry: &Registry,
    enabled: &[String],
    disabled: &[String],
) -> ToolRouter<Lodestone> {
    // General/aggregated tools (macro-generated) + one granular tool per
    // configured provider.
    let mut router = Lodestone::tool_router();
    for route in provider_tool_routes(registry) {
        router.add_route(route);
    }
    let names: Vec<String> = router
        .list_all()
        .iter()
        .map(|t| t.name.to_string())
        .collect();

    for requested in enabled.iter().chain(disabled.iter()) {
        if !names.contains(requested) {
            tracing::warn!(
                tool = requested.as_str(),
                "unknown tool name in [tools]; ignoring"
            );
        }
    }
    for name in &names {
        let keep = (enabled.is_empty() || enabled.contains(name)) && !disabled.contains(name);
        if !keep {
            router.remove_route(name);
        }
    }
    let active = router
        .list_all()
        .iter()
        .map(|t| t.name.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    tracing::info!("active tools: {active}");
    router
}

/// One direct tool per configured provider, named `<kind>_<id>` (e.g.
/// `web_mojeek`, `code_github`, `qa_stackoverflow`). These bypass the chain and
/// strategy, letting the model target a single source.
fn provider_tool_routes(registry: &Registry) -> Vec<ToolRoute<Lodestone>> {
    let schema = schema_for_type::<ProviderSearchArgs>();
    registry
        .list()
        .into_iter()
        .map(|(kind, id)| {
            let name = format!("{}_{}", kind.as_str(), id);
            let description = format!(
                "Search the `{id}` {} provider directly (bypasses the configured chain and \
                 strategy). Use the general {}_search tool to query all configured {} providers.",
                kind.as_str(),
                kind.as_str(),
                kind.as_str(),
            );
            let tool = Tool::new(name, description, schema.clone());
            ToolRoute::new_dyn(tool, move |ctx| provider_call(ctx, kind, id))
        })
        .collect()
}

/// Handler shared by every per-provider tool: parse args, run that one provider,
/// format like its kind.
fn provider_call<'a>(
    ctx: ToolCallContext<'a, Lodestone>,
    kind: ProviderKind,
    id: &'static str,
) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
    Box::pin(async move {
        let svc = ctx.service;
        let args: ProviderSearchArgs = parse_json_object(ctx.arguments.unwrap_or_default())?;
        let q = SearchQuery {
            text: args.query,
            language: args.language,
            site: args.site,
            limit: clamp(args.max_results, 10, 25),
            render: args.render.unwrap_or(false),
        };
        let hits = svc.registry.run_one(kind, id, &svc.http, &q).await;
        let text = match kind {
            ProviderKind::Web => format_web(&q.text, id, &hits),
            ProviderKind::Code => format_code(&q.text, id, &hits),
            ProviderKind::Qa => {
                let site = q.site.as_deref().unwrap_or("stackoverflow");
                format_qa(&q.text, site, &hits)
            }
        };
        Ok(text_result(text))
    })
}

fn clamp(value: Option<u32>, default: u32, max: u32) -> usize {
    value.unwrap_or(default).clamp(1, max) as usize
}

fn text_result(s: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(s.into())])
}

fn internal(e: anyhow::Error) -> McpError {
    McpError::internal_error(format!("{e:#}"), None)
}

fn invalid(e: impl std::fmt::Display) -> McpError {
    McpError::invalid_params(e.to_string(), None)
}

fn indent(s: &str, prefix: &str) -> String {
    s.lines()
        .map(|l| format!("{prefix}{l}"))
        .collect::<Vec<_>>()
        .join("\n")
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

// ---------------------------------------------------------------------------
// Bootstrap
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lodestone_mcp=info,rmcp=warn".into()),
        )
        .init();

    let cfg = Config::load();
    providers::configure_code_sites(cfg.code.sites.clone());
    browser::configure(browser::BrowserOptions {
        chrome_path: cfg.google.chrome_path.clone(),
        no_sandbox: cfg.google.no_sandbox,
        args: cfg.google.args.clone(),
    });
    let registry = Arc::new(Registry::from_config(&cfg));
    tracing::info!("\n{}", registry.describe());

    let server = Lodestone::new(
        registry,
        cfg.stackexchange.default_site.clone(),
        cfg.stackexchange.key.clone(),
        cfg.stackexchange.allowed_sites.clone(),
        &cfg.tools.enabled,
        &cfg.tools.disabled,
    );
    let ct = CancellationToken::new();

    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().with_cancellation_token(ct.child_token()),
    );

    let app = axum::Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("lodestone-mcp listening on http://{}/mcp", cfg.bind);

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            ct.cancel();
        })
        .await?;

    Ok(())
}
