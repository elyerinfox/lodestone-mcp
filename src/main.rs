//! lodestone-mcp — an MCP server that searches and retrieves code from the web
//! by scraping search engines and public endpoints. No API keys / tokens.
//!
//! Sources are pluggable: each one implements the [`provider::SearchProvider`]
//! trait and is selected/ordered via configuration (see [`config`]). Every tool
//! is a self-contained module under [`skills`]; `main.rs` is bootstrap only.
//!
//! Transport: Streamable HTTP, mounted at `/mcp` (works with LM Studio's
//! `url`-style mcp.json entries and any Streamable-HTTP MCP client).

mod browser;
mod cache;
mod config;
mod hive;
mod provider;
mod providers;
mod skills;
mod util;

use std::sync::Arc;
use std::time::Duration;

use rmcp::{
    handler::server::router::tool::ToolRouter,
    model::*,
    tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ErrorData as McpError, ServerHandler,
};
use tokio_util::sync::CancellationToken;

use config::Config;
use provider::Registry;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36";

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// Shared server state. Skills (`src/skills/`) read this via `SkillCtx::server`;
/// fields and helpers are `pub(crate)` so each skill module can use them without
/// any tool logic living here.
#[derive(Clone)]
pub(crate) struct Lodestone {
    pub(crate) http: reqwest::Client,
    pub(crate) registry: Arc<Registry>,
    pub(crate) default_se_site: Arc<str>,
    pub(crate) se_key: Arc<str>,
    pub(crate) se_allowed: Arc<[String]>,
    /// Optional GitHub token (raises the API rate limit for `github_releases`).
    pub(crate) github_token: Arc<str>,
    /// Caches retrieval-tool output (page text, files, answers) keyed by request.
    /// Separate from the search/hive cache so it never enters peer digests.
    pub(crate) retrieval_cache: Option<Arc<cache::TtlCache>>,
    /// Default / hard-cap characters for the retrieval tools (`[retrieval]`).
    pub(crate) default_chars: usize,
    pub(crate) max_chars: usize,
    /// Kubernetes connection settings (kubeconfig path/context/namespace) for the
    /// `k8s_*` tools.
    pub(crate) k8s: Arc<config::Kubernetes>,
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
    pub(crate) fn k8s_opts(&self) -> skills::kubernetes::Opts {
        skills::kubernetes::Opts {
            kubeconfig: self.k8s.kubeconfig.clone(),
            context: self.k8s.context.clone(),
            namespace: self.k8s.namespace.clone(),
        }
    }

    /// Resolve a requested `max_chars`: the per-call value (or the configured
    /// default), clamped to the configured hard cap.
    pub(crate) fn clamp_chars(&self, requested: Option<u32>) -> usize {
        requested
            .map(|n| n as usize)
            .unwrap_or(self.default_chars)
            .clamp(1, self.max_chars)
    }

    /// Guardrail: is `site` permitted by the configured StackExchange allowlist?
    pub(crate) fn se_site_allowed(&self, site: &str) -> bool {
        self.se_allowed.is_empty() || self.se_allowed.iter().any(|s| s == site)
    }

    /// Look up cached retrieval output for `key`, if caching is enabled.
    pub(crate) fn retrieval_get(&self, key: &str) -> Option<String> {
        self.retrieval_cache.as_ref()?.get(key)
    }

    /// Cache non-empty retrieval output for `key` (failures/empties are skipped so
    /// they can be retried).
    pub(crate) fn retrieval_put(&self, key: String, value: &str) {
        if value.is_empty() {
            return;
        }
        if let Some(c) = &self.retrieval_cache {
            c.put(key, value.to_string());
        }
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
                - rfc_get / rfc_search: fetch an IETF RFC's full text by number, or search RFCs by title (keyless).\n\
                - standards_search: search published standards (IEEE/SAE/NIST/ISO/…) via Crossref (metadata; \
                IEEE/SAE paywalled, NIST free).\n\
                - arxiv_search / arxiv_get: search arXiv papers, or get one by id (free PDF → read_pdf for full text).\n\
                - hf_search / hf_model: search the Hugging Face Hub (models/datasets) or get a model's metadata.\n\
                - wikipedia_search / wikipedia_summary: search Wikipedia, or read an article (lead or full); lang configurable.\n\
                - kernel_releases: current Linux kernel releases (mainline/stable/longterm) from kernel.org.\n\
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
                - json_query / json_format / yaml_to_json / json_to_yaml: parse, search, and \
                convert JSON/YAML (local).\n\
                - regex_search / regex_replace: match and substitute with regular expressions (local).\n\
                - math_eval / math_solve: evaluate a math expression, or solve a linear/quadratic \
                equation in x (local).\n\
                - convert_units: convert between units (length/mass/volume/area/speed/time/data/temperature).\n\
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
// Helpers
// ---------------------------------------------------------------------------

/// The effective tool denylist: the configured `[tools].disabled` plus whatever
/// the skills' own gating declares off for the current config (see
/// `skills::disabled_by_config`). No skill names are hardcoded here.
fn effective_disabled(cfg: &Config) -> Vec<String> {
    let mut disabled = cfg.tools.disabled.clone();
    disabled.extend(skills::disabled_by_config(cfg));
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
    for route in skills::all_routes(registry) {
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

pub(crate) fn clamp(value: Option<u32>, default: u32, max: u32) -> usize {
    value.unwrap_or(default).clamp(1, max) as usize
}

pub(crate) fn text_result(s: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(s.into())])
}

pub(crate) fn internal(e: anyhow::Error) -> McpError {
    McpError::internal_error(format!("{e:#}"), None)
}

pub(crate) fn invalid(e: impl std::fmt::Display) -> McpError {
    McpError::invalid_params(e.to_string(), None)
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
