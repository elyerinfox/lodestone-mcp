//! lodestone-mcp — an MCP server that searches and retrieves code from the web
//! by scraping search engines and public endpoints. No API keys / tokens.
//!
//! Sources are pluggable: each one implements the [`provider::SearchProvider`]
//! trait and is selected/ordered via configuration (see [`config`]). Retrieval
//! of a specific resource lives in [`retrieve`].
//!
//! Transport: Streamable HTTP, mounted at `/mcp` (works with LM Studio's
//! `url`-style mcp.json entries and any Streamable-HTTP MCP client).

mod artifacthub;
mod browser;
mod cache;
mod config;
mod docker;
mod hive;
mod k8s;
mod oci;
mod provider;
mod providers;
mod retrieve;
mod translate;
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
struct DocsSearchArgs {
    /// What to look for — a library/package name, API, or documentation topic.
    query: String,
    /// Maximum number of results to return. Default 10, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
    /// Fetch the framework-doc site searches through a real headless browser
    /// (executes JS, can bypass rate-limits) instead of plain HTTP. Slower; needs
    /// a local Chrome/Chromium. Ignored by the JSON registry providers.
    #[serde(default)]
    render: Option<bool>,
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GithubReleasesArgs {
    /// A GitHub repo as `owner/repo` or a github.com URL.
    repo: String,
    /// Max releases to return (newest first). Default 5, capped 30.
    #[serde(default)]
    max_results: Option<u32>,
    /// Include pre-releases and drafts (default false = stable releases only).
    #[serde(default)]
    include_prereleases: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GithubUserArgs {
    /// A GitHub username or org login (e.g. `rust-lang`, `@octocat`, or a
    /// github.com/<user> URL).
    user: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GithubRepoArgs {
    /// A GitHub repo as `owner/repo` or a github.com URL.
    repo: String,
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerSearchArgs {
    /// What to search for on Docker Hub (image name, keyword).
    query: String,
    /// Maximum number of results to return. Default 10, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerImageArgs {
    /// A Docker Hub image: `nginx`, `library/nginx`, or `bitnami/redis` (an
    /// optional `:tag` is ignored — this reports the repository).
    image: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerTagsArgs {
    /// A Docker Hub image: `nginx`, `library/nginx`, or `grafana/grafana`.
    image: String,
    /// Maximum number of tags to return (newest first). Default 15, capped 50.
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct OciTagsArgs {
    /// An image reference on any OCI registry: `nginx`, `ghcr.io/owner/image`,
    /// `quay.io/ns/repo`, `localhost:5000/team/app`.
    reference: String,
    /// Maximum number of tags to return. Default 30, capped 200.
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct OciManifestArgs {
    /// An image reference (with optional `:tag` or `@sha256:…`) on any OCI
    /// registry, e.g. `nginx:1.27`, `ghcr.io/owner/image:latest`.
    reference: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerPsArgs {
    /// Include stopped containers, not just running ones (default false).
    #[serde(default)]
    all: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerNameArgs {
    /// A container name or id.
    container: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerLogsArgs {
    /// A container name or id.
    container: String,
    /// How many trailing log lines to return. Default 200, capped 2000.
    #[serde(default)]
    tail: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerPullArgs {
    /// Image to pull, e.g. `nginx`, `nginx:1.27`, `ghcr.io/owner/image:tag`.
    image: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerRunArgs {
    /// Image to create the container from, e.g. `nginx:alpine`.
    image: String,
    /// Optional container name.
    #[serde(default)]
    name: Option<String>,
    /// Optional command to run (split on whitespace).
    #[serde(default)]
    command: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerRemoveArgs {
    /// A container name or id.
    container: String,
    /// Force-remove a running container (default false).
    #[serde(default)]
    force: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct K8sGetArgs {
    /// Resource kind, e.g. "pods", "deployment", "svc", "nodes", "configmap".
    kind: String,
    /// A specific resource name. Omit to list all of the kind.
    #[serde(default)]
    name: Option<String>,
    /// Namespace (for namespaced kinds). Omit to use the configured/default namespace.
    #[serde(default)]
    namespace: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct K8sDescribeArgs {
    /// Resource kind, e.g. "pod", "deployment", "service".
    kind: String,
    /// The resource name.
    name: String,
    /// Namespace (for namespaced kinds). Omit to use the configured/default namespace.
    #[serde(default)]
    namespace: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct K8sLogsArgs {
    /// Pod name.
    pod: String,
    /// Namespace. Omit to use the configured/default namespace.
    #[serde(default)]
    namespace: Option<String>,
    /// Container name (for multi-container pods). Omit for the default container.
    #[serde(default)]
    container: Option<String>,
    /// Trailing log lines to return. Default 200, capped 2000.
    #[serde(default)]
    tail: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct K8sApplyArgs {
    /// One or more Kubernetes manifests (a "kubefile"): YAML, multi-document
    /// (`---`-separated) allowed. Server-side applied.
    manifest: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct K8sScaleArgs {
    /// Workload kind: "deployment", "statefulset", or "replicaset".
    kind: String,
    /// The workload name.
    name: String,
    /// Desired replica count.
    replicas: i32,
    /// Namespace. Omit to use the configured/default namespace.
    #[serde(default)]
    namespace: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct K8sDeleteArgs {
    /// Resource kind, e.g. "pod", "deployment", "service".
    kind: String,
    /// The resource name.
    name: String,
    /// Namespace (for namespaced kinds). Omit to use the configured/default namespace.
    #[serde(default)]
    namespace: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ArtifactHubArgs {
    /// What to search for (chart/operator/plugin name or keyword).
    query: String,
    /// Optional package-kind filter: helm, olm, krew, falco, opa, kyverno,
    /// gatekeeper, tekton-task, coredns, container, … Omit to search all kinds.
    #[serde(default)]
    kind: Option<String>,
    /// Maximum number of results to return. Default 10, capped at 30.
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TranslateArgs {
    /// The text to translate.
    text: String,
    /// Target language as an ISO-639 code (e.g. "es", "fr", "de", "ja", "zh-CN").
    to: String,
    /// Source language code, or "auto" to detect it (default).
    #[serde(default)]
    from: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DetectLanguageArgs {
    /// The text whose language to detect.
    text: String,
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
    /// Optional GitHub token (raises the API rate limit for `github_releases`).
    github_token: Arc<str>,
    /// Caches retrieval-tool output (page text, files, answers) keyed by request.
    /// Separate from the search/hive cache so it never enters peer digests.
    retrieval_cache: Option<Arc<cache::TtlCache>>,
    /// Default / hard-cap characters for the retrieval tools (`[retrieval]`).
    default_chars: usize,
    max_chars: usize,
    /// Kubernetes connection settings (kubeconfig path/context/namespace) for the
    /// `k8s_*` tools.
    k8s: Arc<config::Kubernetes>,
    // The filtered tool router; `#[tool_handler(router = self.tool_router)]`
    // uses it for both tool listing and dispatch.
    tool_router: ToolRouter<Lodestone>,
}

#[tool_router]
impl Lodestone {
    #[allow(clippy::too_many_arguments)]
    fn new(
        registry: Arc<Registry>,
        default_se_site: String,
        se_key: String,
        se_allowed: Vec<String>,
        github_token: String,
        timeout_secs: u64,
        retrieval_cache: Option<Arc<cache::TtlCache>>,
        default_chars: usize,
        max_chars: usize,
        k8s: config::Kubernetes,
        tools_enabled: &[String],
        tools_disabled: &[String],
    ) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(Duration::from_secs(timeout_secs.max(1)))
            .build()
            .expect("failed to build HTTP client");
        let tool_router = build_tool_router(&registry, tools_enabled, tools_disabled);
        Self {
            http,
            registry,
            default_se_site: default_se_site.into(),
            se_key: se_key.into(),
            se_allowed: se_allowed.into(),
            github_token: github_token.into(),
            retrieval_cache,
            default_chars: default_chars.max(1),
            max_chars: max_chars.max(1),
            k8s: Arc::new(k8s),
            tool_router,
        }
    }

    /// Build per-call Kubernetes connection options from the stored config.
    fn k8s_opts(&self) -> k8s::Opts {
        k8s::Opts {
            kubeconfig: self.k8s.kubeconfig.clone(),
            context: self.k8s.context.clone(),
            namespace: self.k8s.namespace.clone(),
        }
    }

    /// Resolve a requested `max_chars`: the per-call value (or the configured
    /// default), clamped to the configured hard cap.
    fn clamp_chars(&self, requested: Option<u32>) -> usize {
        requested
            .map(|n| n as usize)
            .unwrap_or(self.default_chars)
            .clamp(1, self.max_chars)
    }

    /// Guardrail: is `site` permitted by the configured StackExchange allowlist?
    fn se_site_allowed(&self, site: &str) -> bool {
        self.se_allowed.is_empty() || self.se_allowed.iter().any(|s| s == site)
    }

    /// Look up cached retrieval output for `key`, if caching is enabled.
    fn retrieval_get(&self, key: &str) -> Option<String> {
        self.retrieval_cache.as_ref()?.get(key)
    }

    /// Cache non-empty retrieval output for `key` (failures/empties are skipped so
    /// they can be retried).
    fn retrieval_put(&self, key: String, value: &str) {
        if value.is_empty() {
            return;
        }
        if let Some(c) = &self.retrieval_cache {
            c.put(key, value.to_string());
        }
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
        description = "Search developer documentation and package registries (crates.io, npm, MDN, \
        …) — no API key. Returns matching packages/pages with name, version, URL and description. \
        Use for finding a library or an API reference; then `fetch_page` to read a result."
    )]
    async fn docs_search(
        &self,
        Parameters(args): Parameters<DocsSearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let q = SearchQuery {
            text: args.query.clone(),
            language: None,
            site: None,
            limit: clamp(args.max_results, 10, 25),
            render: args.render.unwrap_or(false),
        };
        let (hits, engine) = self
            .registry
            .search(ProviderKind::Docs, &self.http, &q)
            .await;
        if hits.is_empty() {
            return Ok(text_result(format!(
                "No documentation results for: {}",
                args.query
            )));
        }
        Ok(text_result(format_docs(&args.query, &engine, &hits)))
    }

    #[tool(
        description = "Fetch a web page over plain HTTP and return its readable text (HTML \
        stripped). The default way to read a page (docs, blogs, articles). Output is truncated to \
        a character budget — if the text ends with a '[... truncated ...]' marker and you need \
        more, call again with a larger `max_chars`. If it fails or comes back empty (JS-heavy/SPA), \
        try `render_page`; for a page that's down/changed/blocked, try `wayback_fetch`."
    )]
    async fn fetch_page(
        &self,
        Parameters(args): Parameters<FetchPageArgs>,
    ) -> Result<CallToolResult, McpError> {
        let max = self.clamp_chars(args.max_chars);
        let key = format!("page|{max}|{}", args.url);
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let text = retrieve::fetch_readable(&self.http, &args.url, max)
            .await
            .map_err(internal)?;
        let out = format!("Source: {}\n\n{}", args.url, text);
        if !text.is_empty() {
            self.retrieval_put(key, &out);
        }
        Ok(text_result(out))
    }

    #[tool(
        description = "Fetch a web page through a real headless browser (executes JavaScript) and \
        return its readable text. Use for JS-heavy/SPA pages, or when `fetch_page` is empty or \
        blocked. Output is truncated to a character budget — pass a larger `max_chars` if the text \
        is cut off. Slower than fetch_page and needs a local Chrome/Chromium at runtime."
    )]
    async fn render_page(
        &self,
        Parameters(args): Parameters<RenderPageArgs>,
    ) -> Result<CallToolResult, McpError> {
        use crate::browser::PageRenderer;
        let max = self.clamp_chars(args.max_chars);
        let key = format!("render|{max}|{}", args.url);
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let html = browser::shared_global()
            .render(&args.url)
            .await
            .map_err(internal)?;
        let text = util::truncate_chars(&util::html_to_text(&html), max);
        let out = format!("Source (rendered): {}\n\n{}", args.url, text);
        if !text.is_empty() {
            self.retrieval_put(key, &out);
        }
        Ok(text_result(out))
    }

    #[tool(
        description = "Fetch a page from the Internet Archive Wayback Machine (keyless). Returns \
        the readable text of the closest archived snapshot. Useful when a page is down, paywalled, \
        changed, or blocking automated access, or to view a historical version. Output is truncated \
        to a character budget — pass a larger `max_chars` to get more."
    )]
    async fn wayback_fetch(
        &self,
        Parameters(args): Parameters<WaybackFetchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let max = self.clamp_chars(args.max_chars);
        let key = format!(
            "wayback|{max}|{}|{}",
            args.timestamp.as_deref().unwrap_or(""),
            args.url
        );
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let (snapshot, text) =
            retrieve::wayback_fetch(&self.http, &args.url, args.timestamp.as_deref(), max)
                .await
                .map_err(internal)?;
        let out = format!("Source (archived): {snapshot}\n\n{text}");
        if !text.is_empty() {
            self.retrieval_put(key, &out);
        }
        Ok(text_result(out))
    }

    #[tool(
        description = "Render a web page to a PDF file locally via the headless browser (no \
        external service). Saves to `path`, or a temp file if omitted, and returns the saved path. \
        Needs a local Chrome/Chromium at runtime."
    )]
    async fn webpage_to_pdf(
        &self,
        Parameters(args): Parameters<WebpageToPdfArgs>,
    ) -> Result<CallToolResult, McpError> {
        use crate::browser::PageRenderer;
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
        std::fs::write(&path, &bytes)
            .map_err(|e| internal(anyhow::anyhow!("could not write '{}': {e}", path.display())))?;
        Ok(text_result(format!(
            "Saved {} ({} bytes) from {}",
            path.display(),
            bytes.len(),
            args.url
        )))
    }

    #[tool(
        description = "Read a PDF and return its text, extracted locally (no external service). \
        `source` is an absolute URL or a local file path. Scanned/image-only PDFs (no text layer) \
        return an error rather than text."
    )]
    async fn read_pdf(
        &self,
        Parameters(args): Parameters<ReadPdfArgs>,
    ) -> Result<CallToolResult, McpError> {
        let max = self.clamp_chars(args.max_chars);
        let src = args.source.trim().to_string();
        let key = format!("readpdf|{max}|{src}");
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let bytes: Vec<u8> = if src.starts_with("http://") || src.starts_with("https://") {
            self.http
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
            std::fs::read(&src).map_err(|e| invalid(format!("could not read file '{src}': {e}")))?
        };
        let text = retrieve::extract_pdf_text(bytes, max)
            .await
            .map_err(internal)?;
        let out = format!("PDF: {src}\n\n{text}");
        self.retrieval_put(key, &out);
        Ok(text_result(out))
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
        let key = format!(
            "file|{}|{}|{}",
            args.target,
            args.start_line.unwrap_or(0),
            args.end_line.unwrap_or(0)
        );
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
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

        let out = format!("File: {url}\n\n{content}");
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "List a GitHub repository's releases (newest first): tag, name, date, and \
        release notes. Accepts `owner/repo` or a github.com URL. Keyless (set [github].token to \
        raise the API rate limit). Use for changelogs or 'what changed in version X'."
    )]
    async fn github_releases(
        &self,
        Parameters(args): Parameters<GithubReleasesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let repo = retrieve::github_owner_repo(&args.repo)
            .ok_or_else(|| invalid(format!("not a GitHub owner/repo: '{}'", args.repo)))?;
        let max = clamp(args.max_results, 5, 30);
        let prereleases = args.include_prereleases.unwrap_or(false);
        let key = format!("ghrel|{repo}|{max}|{prereleases}");
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        // Pre-releases/drafts are filtered client-side, so over-fetch when excluding them.
        let per = if prereleases { max } else { (max * 3).min(100) }.to_string();
        let v = retrieve::github_api(
            &self.http,
            &format!("/repos/{repo}/releases"),
            &self.github_token,
            &[("per_page", per.as_str())],
        )
        .await
        .map_err(internal)?;

        let empty = Vec::new();
        let mut out = format!("Releases for {repo}:\n");
        let mut shown = 0usize;
        for r in v.as_array().unwrap_or(&empty) {
            let pre = r
                .get("prerelease")
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            let draft = r.get("draft").and_then(|x| x.as_bool()).unwrap_or(false);
            if !prereleases && (pre || draft) {
                continue;
            }
            let tag = r.get("tag_name").and_then(|x| x.as_str()).unwrap_or("");
            let name = r
                .get("name")
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(tag);
            let date = r
                .get("published_at")
                .and_then(|x| x.as_str())
                .and_then(|d| d.get(..10))
                .unwrap_or("");
            let url = r.get("html_url").and_then(|x| x.as_str()).unwrap_or("");
            let body = r.get("body").and_then(|x| x.as_str()).unwrap_or("").trim();
            shown += 1;
            out.push_str(&format!(
                "\n{shown}. {name} ({tag}){} — {date}\n   {url}\n",
                if pre { " [prerelease]" } else { "" }
            ));
            if !body.is_empty() {
                out.push_str(&indent(&util::truncate_chars(body, 4000), "   "));
                out.push('\n');
            }
            if shown >= max {
                break;
            }
        }
        if shown == 0 {
            return Ok(text_result(format!(
                "No {}releases found for {repo}.",
                if prereleases { "" } else { "stable " }
            )));
        }
        let out = util::truncate_chars(&out, self.max_chars);
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "Get a GitHub user's or org's public profile: name, bio, company, location, \
        blog, public repo count, followers. Accepts a username/login or github.com URL. Keyless."
    )]
    async fn github_user(
        &self,
        Parameters(args): Parameters<GithubUserArgs>,
    ) -> Result<CallToolResult, McpError> {
        let user = retrieve::github_user_login(&args.user)
            .ok_or_else(|| invalid(format!("not a GitHub username: '{}'", args.user)))?;
        let key = format!("ghuser|{user}");
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let v = retrieve::github_api(
            &self.http,
            &format!("/users/{user}"),
            &self.github_token,
            &[],
        )
        .await
        .map_err(internal)?;
        let out = format_github_user(&v, &user);
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "Get a GitHub repository's metadata: description, stars, forks, primary \
        language, topics, license, default branch, homepage, and timestamps. Accepts `owner/repo` \
        or a github.com URL. Keyless."
    )]
    async fn github_repo(
        &self,
        Parameters(args): Parameters<GithubRepoArgs>,
    ) -> Result<CallToolResult, McpError> {
        let repo = retrieve::github_owner_repo(&args.repo)
            .ok_or_else(|| invalid(format!("not a GitHub owner/repo: '{}'", args.repo)))?;
        let key = format!("ghrepo|{repo}");
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let v = retrieve::github_api(
            &self.http,
            &format!("/repos/{repo}"),
            &self.github_token,
            &[],
        )
        .await
        .map_err(internal)?;
        let out = format_github_repo(&v, &repo);
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "Search the configured Q&A providers (currently the StackExchange network: \
        StackOverflow, Server Fault, Super User, Ask Ubuntu, …). Returns matching questions with \
        score, answer count and links. Uses the keyless API by default; set render=true to scrape \
        via a headless browser (no API quota). To search a single site directly use the \
        per-provider tool qa_stackoverflow; use qa_stackoverflow_answers to read the actual answers."
    )]
    async fn qa_search(
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
        description = "Read a StackOverflow/StackExchange question body and its top answers (by \
        votes), including any code blocks. Accepts a question URL or numeric id. Provider-specific \
        to the StackExchange network."
    )]
    async fn qa_stackoverflow_answers(
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

        let key = format!("se_answers|{site}|{max}|{qid}");
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }

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

        let out = util::truncate_chars(&out, self.max_chars);
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "Get the current date and time from the system clock — local time (with UTC \
        offset), UTC, and the Unix timestamp. Use whenever you need to know 'now'; the model's \
        training data has no current time."
    )]
    async fn datetime(
        &self,
        Parameters(args): Parameters<DatetimeArgs>,
    ) -> Result<CallToolResult, McpError> {
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
    }

    #[tool(
        description = "Compute the difference between two dates/times: days (and approximate \
        years), hours, and a human 'ago / from now'. Accepts ISO YYYY-MM-DD, RFC3339, or a Unix \
        timestamp; `to` defaults to now. Use to judge recency — e.g. how long ago a release came out."
    )]
    async fn date_diff(
        &self,
        Parameters(args): Parameters<DateDiffArgs>,
    ) -> Result<CallToolResult, McpError> {
        use chrono::{SecondsFormat, Utc};
        let from = parse_dt(&args.from)
            .ok_or_else(|| invalid(format!("could not parse date/time: '{}'", args.from)))?;
        let to_str = args.to.as_deref().map(str::trim).filter(|s| !s.is_empty());
        let to = match to_str {
            Some(s) => {
                parse_dt(s).ok_or_else(|| invalid(format!("could not parse date/time: '{s}'")))?
            }
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
    }

    #[tool(
        description = "Convert a date/time to another timezone. Accepts ISO/RFC3339, a bare \
        YYYY-MM-DD, or a Unix timestamp; `to_tz` is the target IANA zone (e.g. Asia/Tokyo). For \
        inputs without an offset, `from_tz` says how to interpret them (default UTC)."
    )]
    async fn time_convert(
        &self,
        Parameters(args): Parameters<TimeConvertArgs>,
    ) -> Result<CallToolResult, McpError> {
        use chrono::{SecondsFormat, TimeZone, Utc};
        let to_tz = parse_tz(&args.to_tz)
            .ok_or_else(|| invalid(format!("unknown timezone '{}'", args.to_tz)))?;
        // Resolve the input to an absolute instant: use its offset/unix directly,
        // else interpret the naive time in `from_tz` (default UTC).
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
                    .ok_or_else(|| invalid("that local time is ambiguous or invalid in from_tz"))?
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
    }

    #[tool(
        description = "Search Docker Hub for container images (keyless). Returns name, official/\
        verified status, stars, pull count, and a short description. Use docker_image for one \
        image's details, docker_tags to list its tags."
    )]
    async fn docker_search(
        &self,
        Parameters(args): Parameters<DockerSearchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let limit = clamp(args.max_results, 10, 25);
        let key = format!("docker_search|{limit}|{}", args.query);
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let v = oci::hub_search(&self.http, &args.query, limit)
            .await
            .map_err(internal)?;
        let out = format_docker_search(&args.query, &v, limit);
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "Get a Docker Hub repository's details (keyless): description, stars, pull \
        count, last-updated date, and the long description. Accepts `nginx`, `library/nginx`, or \
        `org/image`."
    )]
    async fn docker_image(
        &self,
        Parameters(args): Parameters<DockerImageArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = oci::parse_ref(&args.image).map_err(|e| invalid(e.to_string()))?;
        let (ns, repo) = r.hub_namespace_repo().ok_or_else(|| {
            invalid(format!(
                "'{}' is not a Docker Hub image; use oci_manifest for other registries",
                args.image
            ))
        })?;
        let key = format!("docker_image|{ns}/{repo}");
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let v = oci::hub_repo(&self.http, &ns, &repo)
            .await
            .map_err(internal)?;
        let out = format_docker_image(&v, &ns, &repo);
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "List a Docker Hub image's tags (keyless), newest first, with compressed \
        size, last-pushed date, and architectures. Accepts `nginx`, `library/nginx`, or \
        `org/image`. For non-Docker-Hub registries use oci_tags."
    )]
    async fn docker_tags(
        &self,
        Parameters(args): Parameters<DockerTagsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let limit = clamp(args.max_results, 15, 50);
        let r = oci::parse_ref(&args.image).map_err(|e| invalid(e.to_string()))?;
        let (ns, repo) = r.hub_namespace_repo().ok_or_else(|| {
            invalid(format!(
                "'{}' is not a Docker Hub image; use oci_tags for other registries",
                args.image
            ))
        })?;
        let key = format!("docker_tags|{limit}|{ns}/{repo}");
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let v = oci::hub_tags(&self.http, &ns, &repo, limit)
            .await
            .map_err(internal)?;
        let out = format_docker_tags(&v, &ns, &repo);
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "List tags for an image on ANY OCI registry (keyless, anonymous pull): \
        Docker Hub, GHCR (ghcr.io), Quay (quay.io), or a self-hosted registry. Accepts `nginx`, \
        `ghcr.io/owner/image`, `quay.io/ns/repo`. Use oci_manifest to inspect one tag's platforms."
    )]
    async fn oci_tags(
        &self,
        Parameters(args): Parameters<OciTagsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let limit = clamp(args.max_results, 30, 200);
        let r = oci::parse_ref(&args.reference).map_err(|e| invalid(e.to_string()))?;
        let key = format!("oci_tags|{limit}|{}/{}", r.registry_host, r.repository);
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let (name, tags) = oci::list_tags(&self.http, &r, limit)
            .await
            .map_err(internal)?;
        if tags.is_empty() {
            return Ok(text_result(format!("No tags found for {}.", r.display())));
        }
        let out = format!(
            "Tags for {}/{name} ({} shown):\n{}",
            r.registry_host,
            tags.len(),
            tags.iter()
                .map(|t| format!("  {t}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "Inspect an image's manifest on ANY OCI registry (keyless, anonymous pull). \
        For a multi-arch image, lists the platforms (os/arch); for a single image, the layer count, \
        total compressed size, and config digest. Accepts `nginx:1.27`, `ghcr.io/owner/image@sha256:…`."
    )]
    async fn oci_manifest(
        &self,
        Parameters(args): Parameters<OciManifestArgs>,
    ) -> Result<CallToolResult, McpError> {
        let r = oci::parse_ref(&args.reference).map_err(|e| invalid(e.to_string()))?;
        let key = format!("oci_manifest|{}", r.display());
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let m = oci::manifest(&self.http, &r).await.map_err(internal)?;
        let mut out = format!(
            "Manifest for {}\n  media type: {}",
            r.display(),
            m.media_type
        );
        if let Some(d) = &m.digest {
            out.push_str(&format!("\n  digest: {d}"));
        }
        if !m.platforms.is_empty() {
            out.push_str(&format!(
                "\n  multi-arch ({} platforms): {}",
                m.platforms.len(),
                m.platforms.join(", ")
            ));
        } else {
            out.push_str(&format!(
                "\n  layers: {} ({})",
                m.layers,
                human_size(m.total_size)
            ));
            if let Some(c) = &m.config_digest {
                out.push_str(&format!("\n  config: {c}"));
            }
        }
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "Search Artifact Hub (keyless) — the index of Kubernetes-ecosystem packages: \
        Helm charts, Operators, krew plugins, Falco/OPA/Kyverno/Gatekeeper policies, Tekton tasks, \
        and more. Optional `kind` filter (e.g. helm, olm, krew). Returns name, version, stars, \
        publisher, and link."
    )]
    async fn artifacthub_search(
        &self,
        Parameters(args): Parameters<ArtifactHubArgs>,
    ) -> Result<CallToolResult, McpError> {
        let limit = clamp(args.max_results, 10, 30);
        let kind = args
            .kind
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let key = format!("artifacthub|{limit}|{}|{}", kind.unwrap_or(""), args.query);
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let v = artifacthub::search(&self.http, &args.query, kind, limit)
            .await
            .map_err(internal)?;
        let out = format_artifacthub(&args.query, kind, &v, limit);
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    // --- Local Docker daemon (gated by [docker]; see src/docker.rs) ----------

    #[tool(
        description = "List containers on the LOCAL Docker daemon (running by default; pass \
        all=true to include stopped). Talks to the daemon directly — no docker CLI. (Distinct from \
        docker_search, which searches Docker Hub.)"
    )]
    async fn docker_ps(
        &self,
        Parameters(args): Parameters<DockerPsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = docker::ps(args.all.unwrap_or(false))
            .await
            .map_err(internal)?;
        Ok(text_result(out))
    }

    #[tool(description = "List images stored on the LOCAL Docker daemon (id, tags, size).")]
    async fn docker_images(&self) -> Result<CallToolResult, McpError> {
        Ok(text_result(docker::images().await.map_err(internal)?))
    }

    #[tool(
        description = "Inspect a LOCAL Docker container (full JSON: config, state, mounts, \
        networks). Accepts a container name or id."
    )]
    async fn docker_inspect(
        &self,
        Parameters(args): Parameters<DockerNameArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = docker::inspect(&args.container).await.map_err(internal)?;
        Ok(text_result(util::truncate_chars(&out, self.max_chars)))
    }

    #[tool(
        description = "Read a LOCAL Docker container's logs (stdout+stderr, last `tail` lines). \
        Accepts a container name or id."
    )]
    async fn docker_logs(
        &self,
        Parameters(args): Parameters<DockerLogsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let tail = clamp(args.tail, 200, 2000);
        let out = docker::logs(&args.container, tail)
            .await
            .map_err(internal)?;
        Ok(text_result(util::truncate_chars(&out, self.max_chars)))
    }

    #[tool(
        description = "Show the LOCAL Docker daemon's version and a summary of its state \
        (containers, images, os/arch)."
    )]
    async fn docker_info(&self) -> Result<CallToolResult, McpError> {
        Ok(text_result(docker::info().await.map_err(internal)?))
    }

    #[tool(
        description = "Pull an image onto the LOCAL Docker daemon, e.g. `nginx:1.27` or \
        `ghcr.io/owner/image:tag`."
    )]
    async fn docker_pull(
        &self,
        Parameters(args): Parameters<DockerPullArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = docker::pull(&args.image).await.map_err(internal)?;
        Ok(text_result(out))
    }

    #[tool(
        description = "Create and start a container on the LOCAL Docker daemon from an image, \
        with an optional name and command. Pulls the image first if needed."
    )]
    async fn docker_run(
        &self,
        Parameters(args): Parameters<DockerRunArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = docker::run(&args.image, args.name.as_deref(), args.command.as_deref())
            .await
            .map_err(internal)?;
        Ok(text_result(out))
    }

    #[tool(
        description = "Start an existing (stopped) container on the LOCAL Docker daemon. \
        Accepts a container name or id."
    )]
    async fn docker_start(
        &self,
        Parameters(args): Parameters<DockerNameArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = docker::start(&args.container).await.map_err(internal)?;
        Ok(text_result(out))
    }

    #[tool(
        description = "Stop a running container on the LOCAL Docker daemon. Destructive — only \
        available when [docker].allow_destructive is set. Accepts a container name or id."
    )]
    async fn docker_stop(
        &self,
        Parameters(args): Parameters<DockerNameArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = docker::stop(&args.container).await.map_err(internal)?;
        Ok(text_result(out))
    }

    #[tool(
        description = "Remove a container from the LOCAL Docker daemon (optionally force a \
        running one). Destructive — only available when [docker].allow_destructive is set."
    )]
    async fn docker_remove(
        &self,
        Parameters(args): Parameters<DockerRemoveArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = docker::remove(&args.container, args.force.unwrap_or(false))
            .await
            .map_err(internal)?;
        Ok(text_result(out))
    }

    // --- Kubernetes cluster (gated by [kubernetes]; see src/k8s.rs) ----------

    #[tool(
        description = "List the kubeconfig contexts and the current one (no cluster contact). \
        Use to see which clusters are configured."
    )]
    async fn k8s_contexts(&self) -> Result<CallToolResult, McpError> {
        let out = k8s::contexts(&self.k8s_opts()).map_err(internal)?;
        Ok(text_result(out))
    }

    #[tool(
        description = "Get Kubernetes resources from the cluster: a single named object (full \
        JSON) or a list of a kind. `kind` accepts kubectl names (pods, deploy, svc, nodes, …). \
        Reads your kubeconfig; no kubectl."
    )]
    async fn k8s_get(
        &self,
        Parameters(args): Parameters<K8sGetArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = k8s::get(
            &self.k8s_opts(),
            &args.kind,
            args.name.as_deref(),
            args.namespace.as_deref(),
        )
        .await
        .map_err(internal)?;
        Ok(text_result(util::truncate_chars(&out, self.max_chars)))
    }

    #[tool(
        description = "Describe one Kubernetes resource (full JSON of the named object). \
        Reads your kubeconfig; no kubectl."
    )]
    async fn k8s_describe(
        &self,
        Parameters(args): Parameters<K8sDescribeArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = k8s::describe(
            &self.k8s_opts(),
            &args.kind,
            &args.name,
            args.namespace.as_deref(),
        )
        .await
        .map_err(internal)?;
        Ok(text_result(util::truncate_chars(&out, self.max_chars)))
    }

    #[tool(
        description = "Read a Kubernetes pod's logs (last `tail` lines; optional container). \
        Reads your kubeconfig; no kubectl."
    )]
    async fn k8s_logs(
        &self,
        Parameters(args): Parameters<K8sLogsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let tail = clamp(args.tail, 200, 2000);
        let out = k8s::logs(
            &self.k8s_opts(),
            &args.pod,
            args.namespace.as_deref(),
            args.container.as_deref(),
            tail,
        )
        .await
        .map_err(internal)?;
        Ok(text_result(util::truncate_chars(&out, self.max_chars)))
    }

    #[tool(
        description = "Apply a Kubernetes manifest ('kubefile') to the cluster via server-side \
        apply. `manifest` is YAML (multi-document allowed). Creates or updates the objects. Reads \
        your kubeconfig; no kubectl."
    )]
    async fn k8s_apply(
        &self,
        Parameters(args): Parameters<K8sApplyArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = k8s::apply(&self.k8s_opts(), &args.manifest)
            .await
            .map_err(internal)?;
        Ok(text_result(out))
    }

    #[tool(
        description = "Scale a Kubernetes workload (deployment/statefulset/replicaset) to a \
        replica count. Reads your kubeconfig; no kubectl."
    )]
    async fn k8s_scale(
        &self,
        Parameters(args): Parameters<K8sScaleArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = k8s::scale(
            &self.k8s_opts(),
            &args.kind,
            &args.name,
            args.replicas,
            args.namespace.as_deref(),
        )
        .await
        .map_err(internal)?;
        Ok(text_result(out))
    }

    #[tool(
        description = "Delete a Kubernetes resource by kind + name. Destructive — only \
        available when [kubernetes].allow_destructive is set. Reads your kubeconfig; no kubectl."
    )]
    async fn k8s_delete(
        &self,
        Parameters(args): Parameters<K8sDeleteArgs>,
    ) -> Result<CallToolResult, McpError> {
        let out = k8s::delete(
            &self.k8s_opts(),
            &args.kind,
            &args.name,
            args.namespace.as_deref(),
        )
        .await
        .map_err(internal)?;
        Ok(text_result(out))
    }

    #[tool(
        description = "Translate text into another language with Google Translate (keyless, no API \
        key). `to` is an ISO-639 target code (es, fr, de, ja, zh-CN, …); `from` defaults to \
        auto-detect. Returns the translation and the detected source language."
    )]
    async fn translate(
        &self,
        Parameters(args): Parameters<TranslateArgs>,
    ) -> Result<CallToolResult, McpError> {
        let to = args.to.trim();
        if to.is_empty() {
            return Err(invalid("`to` (target language code) is required"));
        }
        let from = args.from.as_deref().map(str::trim).unwrap_or("auto");
        let key = format!("translate|{from}|{to}|{}", args.text);
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let t = translate::translate(&self.http, &args.text, to, from)
            .await
            .map_err(internal)?;
        let detected = if t.source_lang.is_empty() {
            from.to_string()
        } else {
            t.source_lang
        };
        let out = format!("Translation ({detected} → {to}):\n{}", t.text);
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "Detect the language of a piece of text using Google Translate (keyless). \
        Returns the detected ISO-639 language code."
    )]
    async fn detect_language(
        &self,
        Parameters(args): Parameters<DetectLanguageArgs>,
    ) -> Result<CallToolResult, McpError> {
        let key = format!("detect|{}", args.text);
        if let Some(cached) = self.retrieval_get(&key) {
            return Ok(text_result(cached));
        }
        let t = translate::translate(&self.http, &args.text, "en", "auto")
            .await
            .map_err(internal)?;
        if t.source_lang.is_empty() {
            return Ok(text_result("Could not detect the language."));
        }
        let out = format!("Detected language: {}", t.source_lang);
        self.retrieval_put(key, &out);
        Ok(text_result(out))
    }

    #[tool(
        description = "List the configured search providers and the order they are tried, for \
        web, code and Q&A. Useful to check which sources are active."
    )]
    async fn list_providers(&self) -> Result<CallToolResult, McpError> {
        Ok(text_result(self.registry.describe()))
    }

    #[tool(
        description = "Show the peer-to-peer hivemind graph: this node's id and its known peers \
        with reputation, reachability, and the mesh edges they advertise. Reports that the \
        hivemind is disabled when [network].enabled is false."
    )]
    async fn hive_status(&self) -> Result<CallToolResult, McpError> {
        Ok(text_result(self.registry.hive_report()))
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
                - docs_search: search docs & package registries (crates.io, npm, MDN) and \
                framework docs (PHP, Laravel, Vue, React, Svelte, …).\n\
                - fetch_repo_file: download a full file from GitHub/GitLab/Gitea by URL or owner/repo/path.\n\
                - github_releases / github_user / github_repo: GitHub release notes, profiles, repo metadata (keyless).\n\
                - fetch_page: get readable text of any URL over plain HTTP.\n\
                - render_page: get readable text of a URL via a headless browser (JS).\n\
                - webpage_to_pdf: save a web page to a local PDF (headless browser).\n\
                - read_pdf: extract text from a PDF (URL or local path), locally.\n\
                - wayback_fetch: read a page's archived snapshot from the Wayback Machine.\n\
                - qa_search: search the configured Q&A providers (StackExchange network).\n\
                - datetime: the current date/time from the system clock (local, UTC, Unix).\n\
                - date_diff: difference between two dates (days/years, 'ago / from now').\n\
                - time_convert: convert a date/time to another IANA timezone.\n\
                - translate / detect_language: Google Translate (keyless) — translate text or \
                detect its language.\n\
                - docker_search / docker_image / docker_tags: Docker Hub image search + metadata + tags (keyless).\n\
                - oci_tags / oci_manifest: list tags / inspect a manifest on any OCI registry (Docker Hub, GHCR, Quay, …).\n\
                - artifacthub_search: search Artifact Hub (Helm charts, Operators, krew, policies, …).\n\
                - docker_ps / docker_images / docker_logs / docker_inspect / docker_info / docker_pull / \
                docker_run / docker_start (+ docker_stop / docker_remove when allowed): control the LOCAL \
                Docker daemon (gated by [docker]).\n\
                - k8s_contexts / k8s_get / k8s_describe / k8s_logs / k8s_apply / k8s_scale \
                (+ k8s_delete when allowed): interact with a Kubernetes cluster via your kubeconfig \
                (gated by [kubernetes]).\n\
                - list_providers: show which sources are active.\n\
                - hive_status: show the peer-to-peer hivemind graph (if enabled).\n\
                Each configured provider also has a direct tool named <kind>_<id> \
                (e.g. web_mojeek, code_github, qa_stackoverflow) to target one source. \
                StackOverflow adds qa_stackoverflow_answers to read a question's top answers (with code).\n\n\
                Typical flow: search (web_search/code_search/docs_search/qa_search) → then retrieve \
                (fetch_repo_file / fetch_page / render_page / qa_stackoverflow_answers) on the best hit."
                    .to_string(),
            )
    }
}

// ---------------------------------------------------------------------------
// Output formatting
// ---------------------------------------------------------------------------

/// Local current date (YYYY-MM-DD) stamped onto result headers so the model can
/// anchor recency instead of guessing — web snippets often omit the year.
fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

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
fn parse_dt(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{TimeZone, Utc};
    parse_instant(s).or_else(|| parse_naive(s).map(|n| Utc.from_utc_datetime(&n)))
}

/// Parse an IANA timezone name (e.g. `America/New_York`, `Asia/Tokyo`, `UTC`).
fn parse_tz(name: &str) -> Option<chrono_tz::Tz> {
    name.trim().parse::<chrono_tz::Tz>().ok()
}

fn format_web(query: &str, engine: &str, hits: &[SearchResult]) -> String {
    let mut out = format!(
        "Web results for \"{query}\" (current date {}; via {engine}):\n",
        now_stamp()
    );
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
    let mut out = format!(
        "Code results for \"{query}\" (current date {}; via {engine}):\n",
        now_stamp()
    );
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

fn format_docs(query: &str, engine: &str, hits: &[SearchResult]) -> String {
    let mut out = format!(
        "Documentation results for \"{query}\" (current date {}; via {engine}):\n",
        now_stamp()
    );
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n   {}\n", i + 1, h.title, h.url));
        if !h.snippet.is_empty() {
            out.push_str(&format!("   {}\n", h.snippet));
        }
    }
    out
}

/// Compact human byte size (e.g. "36.3 MB").
fn human_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

/// Compact human count (e.g. "13.0B", "21.3K") for star/pull tallies.
fn human_count(n: i64) -> String {
    let a = n.unsigned_abs() as f64;
    let (v, suffix) = if a >= 1e9 {
        (a / 1e9, "B")
    } else if a >= 1e6 {
        (a / 1e6, "M")
    } else if a >= 1e3 {
        (a / 1e3, "K")
    } else {
        return n.to_string();
    };
    format!("{}{v:.1}{suffix}", if n < 0 { "-" } else { "" })
}

fn format_docker_search(query: &str, v: &serde_json::Value, limit: usize) -> String {
    let mut out = format!("Docker Hub results for \"{query}\":\n");
    let empty = Vec::new();
    let results = v
        .get("results")
        .and_then(|x| x.as_array())
        .unwrap_or(&empty);
    for (i, r) in results.iter().take(limit).enumerate() {
        let name = r.get("repo_name").and_then(|x| x.as_str()).unwrap_or("");
        let official = r
            .get("is_official")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let stars = r.get("star_count").and_then(|x| x.as_i64()).unwrap_or(0);
        let pulls = r.get("pull_count").and_then(|x| x.as_i64()).unwrap_or(0);
        let desc = r
            .get("short_description")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let url = if official {
            format!("https://hub.docker.com/_/{name}")
        } else {
            format!("https://hub.docker.com/r/{name}")
        };
        out.push_str(&format!(
            "\n{}. {name}{}\n   {url}\n   stars {} · pulls {}\n",
            i + 1,
            if official { " [official]" } else { "" },
            human_count(stars),
            human_count(pulls),
        ));
        if !desc.is_empty() {
            out.push_str(&format!("   {desc}\n"));
        }
    }
    out
}

fn format_docker_image(v: &serde_json::Value, ns: &str, repo: &str) -> String {
    let official = ns == "library";
    let full = if official {
        repo.to_string()
    } else {
        format!("{ns}/{repo}")
    };
    let url = if official {
        format!("https://hub.docker.com/_/{repo}")
    } else {
        format!("https://hub.docker.com/r/{ns}/{repo}")
    };
    let mut out = format!("Docker Hub image: {full}");
    if official {
        out.push_str(" [official]");
    }
    out.push('\n');
    if let Some(d) = v
        .get("description")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        out.push_str(&format!("{d}\n"));
    }
    if let Some(s) = v.get("star_count").and_then(|x| x.as_i64()) {
        out.push_str(&format!("  stars: {}\n", human_count(s)));
    }
    if let Some(p) = v.get("pull_count").and_then(|x| x.as_i64()) {
        out.push_str(&format!("  pulls: {}\n", human_count(p)));
    }
    if let Some(u) = v
        .get("last_updated")
        .and_then(|x| x.as_str())
        .and_then(|d| d.get(..10))
    {
        out.push_str(&format!("  last updated: {u}\n"));
    }
    out.push_str(&format!("  {url}\n"));
    if let Some(full_desc) = v
        .get("full_description")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push('\n');
        out.push_str(&util::truncate_chars(full_desc, 3000));
        out.push('\n');
    }
    out
}

fn format_docker_tags(v: &serde_json::Value, ns: &str, repo: &str) -> String {
    let full = if ns == "library" {
        repo.to_string()
    } else {
        format!("{ns}/{repo}")
    };
    let empty = Vec::new();
    let results = v
        .get("results")
        .and_then(|x| x.as_array())
        .unwrap_or(&empty);
    let mut out = format!("Tags for {full} ({} shown, newest first):\n", results.len());
    for t in results {
        let name = t.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let size = t.get("full_size").and_then(|x| x.as_u64()).unwrap_or(0);
        let pushed = t
            .get("tag_last_pushed")
            .and_then(|x| x.as_str())
            .and_then(|d| d.get(..10))
            .unwrap_or("");
        let archs: Vec<&str> = t
            .get("images")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|im| im.get("architecture").and_then(|x| x.as_str()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        out.push_str(&format!("\n  {name}"));
        let mut facts = Vec::new();
        if size > 0 {
            facts.push(human_size(size));
        }
        if !pushed.is_empty() {
            facts.push(pushed.to_string());
        }
        if !archs.is_empty() {
            facts.push(archs.join("/"));
        }
        if !facts.is_empty() {
            out.push_str(&format!("  ({})", facts.join(" · ")));
        }
    }
    out.push('\n');
    out
}

fn format_artifacthub(
    query: &str,
    kind: Option<&str>,
    v: &serde_json::Value,
    limit: usize,
) -> String {
    let scope = kind.map(|k| format!(" [{k}]")).unwrap_or_default();
    let mut out = format!("Artifact Hub results for \"{query}\"{scope}:\n");
    let empty = Vec::new();
    let packages = v
        .get("packages")
        .and_then(|x| x.as_array())
        .unwrap_or(&empty);
    if packages.is_empty() {
        out.push_str("\n(no packages found)");
        return out;
    }
    for (i, p) in packages.iter().take(limit).enumerate() {
        let name = p.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let version = p.get("version").and_then(|x| x.as_str()).unwrap_or("");
        let stars = p.get("stars").and_then(|x| x.as_i64()).unwrap_or(0);
        let desc = p.get("description").and_then(|x| x.as_str()).unwrap_or("");
        let repo = p.get("repository");
        let kind_slug = repo
            .and_then(|r| r.get("kind"))
            .and_then(|x| x.as_u64())
            .and_then(artifacthub::kind_slug)
            .unwrap_or("package");
        let publisher = repo
            .and_then(|r| {
                r.get("organization_name")
                    .or_else(|| r.get("user_alias"))
                    .or_else(|| r.get("name"))
            })
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let url = artifacthub::package_url(p);
        out.push_str(&format!("\n{}. {name}", i + 1));
        if !version.is_empty() {
            out.push_str(&format!(" {version}"));
        }
        out.push_str(&format!(" [{kind_slug}]"));
        out.push('\n');
        out.push_str(&format!("   {url}\n"));
        let mut facts = Vec::new();
        if !publisher.is_empty() {
            facts.push(format!("by {publisher}"));
        }
        if stars > 0 {
            facts.push(format!("★ {}", human_count(stars)));
        }
        if !facts.is_empty() {
            out.push_str(&format!("   {}\n", facts.join(" · ")));
        }
        if !desc.is_empty() {
            out.push_str(&format!("   {desc}\n"));
        }
    }
    out
}

fn format_github_user(v: &serde_json::Value, fallback: &str) -> String {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).filter(|x| !x.is_empty());
    let n = |k: &str| v.get(k).and_then(|x| x.as_i64());
    let login = s("login").unwrap_or(fallback);
    let kind = s("type").unwrap_or("User");
    let mut out = format!("{kind}: {login}");
    if let Some(name) = s("name") {
        out.push_str(&format!(" ({name})"));
    }
    out.push('\n');
    if let Some(bio) = s("bio") {
        out.push_str(&format!("{bio}\n"));
    }
    let mut facts = Vec::new();
    if let Some(c) = s("company") {
        facts.push(format!("company: {c}"));
    }
    if let Some(l) = s("location") {
        facts.push(format!("location: {l}"));
    }
    if let Some(blog) = s("blog") {
        facts.push(format!("blog: {blog}"));
    }
    if let Some(e) = s("email") {
        facts.push(format!("email: {e}"));
    }
    if let Some(r) = n("public_repos") {
        facts.push(format!("public repos: {r}"));
    }
    if let Some(f) = n("followers") {
        facts.push(format!("followers: {f}"));
    }
    if let Some(f) = n("following") {
        facts.push(format!("following: {f}"));
    }
    if let Some(joined) = s("created_at").and_then(|d| d.get(..10)) {
        facts.push(format!("joined: {joined}"));
    }
    for f in facts {
        out.push_str(&format!("  {f}\n"));
    }
    if let Some(u) = s("html_url") {
        out.push_str(&format!("  {u}\n"));
    }
    out
}

fn format_github_repo(v: &serde_json::Value, fallback: &str) -> String {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).filter(|x| !x.is_empty());
    let n = |k: &str| v.get(k).and_then(|x| x.as_i64());
    let flag = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    let full = s("full_name").unwrap_or(fallback);
    let mut out = full.to_string();
    if flag("archived") {
        out.push_str(" [archived]");
    }
    if flag("fork") {
        out.push_str(" [fork]");
    }
    out.push('\n');
    if let Some(d) = s("description") {
        out.push_str(&format!("{d}\n"));
    }
    let mut facts = Vec::new();
    if let Some(x) = n("stargazers_count") {
        facts.push(format!("stars: {x}"));
    }
    if let Some(x) = n("forks_count") {
        facts.push(format!("forks: {x}"));
    }
    if let Some(x) = n("open_issues_count") {
        facts.push(format!("open issues: {x}"));
    }
    if let Some(lang) = s("language") {
        facts.push(format!("language: {lang}"));
    }
    if let Some(topics) = v.get("topics").and_then(|x| x.as_array()) {
        let t: Vec<&str> = topics.iter().filter_map(|x| x.as_str()).collect();
        if !t.is_empty() {
            facts.push(format!("topics: {}", t.join(", ")));
        }
    }
    if let Some(lic) = v
        .get("license")
        .and_then(|l| l.get("spdx_id"))
        .and_then(|x| x.as_str())
        .filter(|x| !x.is_empty() && *x != "NOASSERTION")
    {
        facts.push(format!("license: {lic}"));
    }
    if let Some(db) = s("default_branch") {
        facts.push(format!("default branch: {db}"));
    }
    if let Some(hp) = s("homepage") {
        facts.push(format!("homepage: {hp}"));
    }
    if let Some(pa) = s("pushed_at").and_then(|d| d.get(..10)) {
        facts.push(format!("last push: {pa}"));
    }
    for f in facts {
        out.push_str(&format!("  {f}\n"));
    }
    if let Some(u) = s("html_url") {
        out.push_str(&format!("  {u}\n"));
    }
    out
}

fn format_qa(query: &str, site: &str, hits: &[SearchResult]) -> String {
    let mut out = format!(
        "{site} results for \"{query}\" (current date {}):\n",
        now_stamp()
    );
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
    out.push_str("\nTip: pass a question URL to qa_stackoverflow_answers to read answers.");
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Local Docker daemon tools, and the destructive subset within them.
const DOCKER_TOOLS: &[&str] = &[
    "docker_ps",
    "docker_images",
    "docker_inspect",
    "docker_logs",
    "docker_info",
    "docker_pull",
    "docker_run",
    "docker_start",
    "docker_stop",
    "docker_remove",
];
const DOCKER_DESTRUCTIVE: &[&str] = &["docker_stop", "docker_remove"];

/// Kubernetes tools, and the destructive subset within them.
const K8S_TOOLS: &[&str] = &[
    "k8s_contexts",
    "k8s_get",
    "k8s_describe",
    "k8s_logs",
    "k8s_apply",
    "k8s_scale",
    "k8s_delete",
];
const K8S_DESTRUCTIVE: &[&str] = &["k8s_delete"];

/// The effective tool denylist: the configured `[tools].disabled`, plus the
/// local-system tools gated off by their family config (whole family when
/// disabled, just the destructive ones when destructive isn't allowed).
fn effective_disabled(cfg: &Config) -> Vec<String> {
    let mut disabled = cfg.tools.disabled.clone();
    let mut deny = |names: &[&str]| disabled.extend(names.iter().map(|s| s.to_string()));
    if !cfg.docker.enabled {
        deny(DOCKER_TOOLS);
    } else if !cfg.docker.allow_destructive {
        deny(DOCKER_DESTRUCTIVE);
    }
    if !cfg.kubernetes.enabled {
        deny(K8S_TOOLS);
    } else if !cfg.kubernetes.allow_destructive {
        deny(K8S_DESTRUCTIVE);
    }
    disabled
}

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
            ProviderKind::Docs => format_docs(&q.text, id, &hits),
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

/// Reject `/mcp` requests lacking the configured `Authorization: Bearer <token>`.
async fn require_bearer(
    axum::extract::State(token): axum::extract::State<Arc<str>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let presented = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));
    match presented {
        Some(t) if util::ct_eq(t.as_bytes(), token.as_bytes()) => next.run(req).await,
        _ => (axum::http::StatusCode::UNAUTHORIZED, "unauthorized\n").into_response(),
    }
}

/// Extract the `Authorization: Bearer <token>` value from request headers.
fn bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
}

/// Hivemind peer endpoints (`/hive/digest`, `/hive/query`), each guarded by the
/// optional `[network].token`. Returns only cached search results — never secrets.
fn hive_routes(hive: Arc<hive::Hive>) -> axum::Router {
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};

    async fn digest(
        State(hive): State<Arc<hive::Hive>>,
        headers: HeaderMap,
    ) -> axum::response::Response {
        if !hive.token_ok(bearer_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        axum::Json(hive.digest()).into_response()
    }

    async fn query(
        State(hive): State<Arc<hive::Hive>>,
        headers: HeaderMap,
        axum::Json(req): axum::Json<hive::QueryReq>,
    ) -> axum::response::Response {
        if !hive.token_ok(bearer_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        // Serve from our cache, or relay one+ hops toward a holder (bounded).
        let hits = hive.answer_query(&req.key, req.ttl, &req.seen).await;
        if hits.is_empty() {
            return StatusCode::NO_CONTENT.into_response();
        }
        axum::Json(hive::QueryResp { hits }).into_response()
    }

    axum::Router::new()
        .route("/hive/digest", get(digest))
        .route("/hive/query", post(query))
        .with_state(hive)
}

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
    // The result cache is shared with the hivemind (which reads/serves from it),
    // so enabling the network implies an active cache even if [cache] is off.
    let cache = (cfg.cache.enabled || cfg.network.enabled).then(|| {
        Arc::new(cache::TtlCache::new(
            cfg.cache.ttl_secs.max(1),
            cfg.cache.max_entries,
        ))
    });
    let hive = cfg.network.enabled.then(|| {
        hive::Hive::new(
            &cfg.network,
            cache.clone().expect("cache exists when network enabled"),
        )
    });
    let registry = Arc::new(Registry::from_config(&cfg, cache.clone(), hive.clone()));
    tracing::info!("\n{}", registry.describe());

    // A separate cache for retrieval-tool output (page text, files, answers), so
    // those entries never enter the search/hive digest.
    let retrieval_cache = cfg.cache.enabled.then(|| {
        Arc::new(cache::TtlCache::new(
            cfg.cache.ttl_secs.max(1),
            cfg.cache.max_entries,
        ))
    });

    // Gate the local-system tool families by their config: when a family is off,
    // hide all its tools; when on but destructive actions aren't allowed, hide
    // just those. (Done by extending the [tools] denylist before the router is
    // built, so the gating reuses the same filtering path.)
    let tools_disabled = effective_disabled(&cfg);

    let server = Lodestone::new(
        registry,
        cfg.stackexchange.default_site.clone(),
        cfg.stackexchange.key.clone(),
        cfg.stackexchange.allowed_sites.clone(),
        cfg.github.token.clone(),
        cfg.search.timeout_secs,
        retrieval_cache,
        cfg.retrieval.default_chars,
        cfg.retrieval.max_chars,
        cfg.kubernetes.clone(),
        &cfg.tools.enabled,
        &tools_disabled,
    );
    let ct = CancellationToken::new();

    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().with_cancellation_token(ct.child_token()),
    );

    // The MCP endpoint, optionally guarded by a bearer token. `/health` is always
    // open so container/orchestrator probes work without credentials.
    let mut mcp = axum::Router::new().nest_service("/mcp", service);
    if !cfg.auth_token.is_empty() {
        let token: Arc<str> = Arc::from(cfg.auth_token.as_str());
        mcp = mcp.layer(axum::middleware::from_fn_with_state(token, require_bearer));
        tracing::info!("MCP endpoint requires bearer authentication");
    }
    let mut app = axum::Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }))
        .merge(mcp);

    // Hivemind: mount peer endpoints and start discovery/sync (opt-in).
    if let Some(h) = &hive {
        let bind_port = cfg
            .bind
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(0);
        app = app.merge(hive_routes(h.clone()));
        h.clone().start(bind_port);
        tracing::info!(
            peers = cfg.network.peers.len(),
            mdns = cfg.network.mdns,
            "hivemind enabled"
        );
    }

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

#[cfg(test)]
mod tests {
    use super::parse_dt;

    #[test]
    fn parse_dt_accepts_common_formats() {
        // bare date → midnight UTC
        let d = parse_dt("2025-05-27").unwrap();
        assert_eq!(
            d.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "2025-05-27T00:00:00Z"
        );
        // RFC3339 with offset → normalized to UTC
        let d = parse_dt("2025-05-27T18:25:00-07:00").unwrap();
        assert_eq!(
            d.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            "2025-05-28T01:25:00Z"
        );
        // unix timestamp
        assert_eq!(parse_dt("0").unwrap().timestamp(), 0);
        // junk
        assert!(parse_dt("not a date").is_none());
    }
}
