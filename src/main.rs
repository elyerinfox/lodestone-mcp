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
mod constellation;
mod galaxy;
mod provider;
mod providers;
mod skills;
mod store;
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

pub(crate) const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
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
    /// Optional NASA api.nasa.gov key (empty → `DEMO_KEY`) for the `nasa_*` tools.
    pub(crate) nasa_key: Arc<str>,
    /// Optional EIA Open Data API key for `eia_*` tools.
    pub(crate) eia_key: Arc<str>,
    /// Serial-port policy (baud/timeout) for the `serial_*` tools.
    pub(crate) serial: Arc<config::Serial>,
    /// Caches retrieval-tool output (page text, files, answers) keyed by request.
    /// Separate from the search/constellation cache so it never enters peer digests.
    pub(crate) retrieval_cache: Option<Arc<cache::TtlCache>>,
    /// Default / hard-cap characters for the retrieval tools (`[retrieval]`).
    pub(crate) default_chars: usize,
    pub(crate) max_chars: usize,
    /// Docker policy (destructive gating) for the `docker_*` tools.
    pub(crate) docker: Arc<config::Docker>,
    /// Kubernetes connection settings (kubeconfig path/context/namespace) for the
    /// `k8s_*` tools.
    pub(crate) k8s: Arc<config::Kubernetes>,
    /// Filesystem roots/permissions for the `fs_*` tools.
    pub(crate) fs: Arc<config::Filesystem>,
    /// Shell-execution policy (allowlist / unrestricted) for `shell_run`.
    pub(crate) shell: Arc<config::Shell>,
    /// Git CLI policy (repo, destructive gating) for `git_run`.
    pub(crate) git: Arc<config::Git>,
    /// Database-skill settings (enabled + allow_destructive). Connections are ad-hoc
    /// (passed per call), so no stored instances/credentials.
    pub(crate) databases: Arc<config::Databases>,
    /// Optional on-disk file store for fetched bytes (the `store_*` tools).
    pub(crate) store: Option<Arc<store::FileStore>>,
    /// Per-session confirmation state for destructive actions (the client-agnostic
    /// alternative to MCP elicitation). Shared across cloned handles.
    pub(crate) guard: skills::guard::Guard,
    /// Background-job registry (model-polled): `task_*` tools spawn long work and
    /// poll for results here. Shared across cloned handles.
    pub(crate) tasks: skills::tasks::Tasks,
    /// Persistent memory & solution-history store (the `memory_*` / `solution_*`
    /// tools). On-disk JSONL under `[memory].dir`. Shared across cloned handles.
    pub(crate) memory: skills::memory::Memory,
    /// Python runner settings (`python_run`).
    pub(crate) python: Arc<config::Python>,
    /// systemd skill settings.
    pub(crate) systemd: Arc<config::Systemd>,
    /// Whole resolved server configuration, shared by `Arc`. Held so
    /// introspection tools (the `features` skill) can report every gateable
    /// family's on/off state and key knobs without dragging individual config
    /// sections into the constructor signature one-by-one.
    pub(crate) cfg: Arc<config::Config>,
    /// The set of tool names the resolved config has gated off. Precomputed
    /// at startup (the source of truth used to build the tool router) so the
    /// `features` skill can map families to "any of these tools hidden?"
    /// without re-running the resolution.
    pub(crate) disabled_tools: Arc<Vec<String>>,
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
        nasa_key: String,
        eia_key: String,
        serial: config::Serial,
        timeout_secs: u64,
        retrieval_cache: Option<Arc<cache::TtlCache>>,
        default_chars: usize,
        max_chars: usize,
        docker: config::Docker,
        k8s: config::Kubernetes,
        fs: config::Filesystem,
        shell: config::Shell,
        git: config::Git,
        databases: config::Databases,
        store: Option<Arc<store::FileStore>>,
        memory: skills::memory::Memory,
        python: config::Python,
        systemd: config::Systemd,
        cfg: Arc<config::Config>,
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
            nasa_key: nasa_key.into(),
            eia_key: eia_key.into(),
            serial: Arc::new(serial),
            retrieval_cache,
            default_chars: default_chars.max(1),
            max_chars: max_chars.max(1),
            docker: Arc::new(docker),
            k8s: Arc::new(k8s),
            fs: Arc::new(fs),
            shell: Arc::new(shell),
            git: Arc::new(git),
            databases: Arc::new(databases),
            store,
            guard: skills::guard::Guard::default(),
            tasks: skills::tasks::Tasks::new(),
            memory,
            python: Arc::new(python),
            systemd: Arc::new(systemd),
            disabled_tools: Arc::new(tools_disabled.to_vec()),
            cfg,
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

    /// Look up cached retrieval output for `key`: the local retrieval cache first,
    /// then (Bloom-gated, so a true miss costs nothing) a constellation peer that has it —
    /// letting one node's fetched/parsed text serve the mesh. Entries are keyed by
    /// hash, matching what the constellation advertises and serves.
    pub(crate) async fn retrieval_get(&self, key: &str) -> Option<String> {
        let hash = crate::constellation::hash_key(key);
        if let Some(c) = &self.retrieval_cache {
            if let Some(v) = c.get(&hash) {
                return Some(v);
            }
        }
        if let Some(constellation) = self.registry.constellation() {
            if let Some(bytes) = constellation.consult_blob_hash(&hash).await {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                if !text.is_empty() {
                    if let Some(c) = &self.retrieval_cache {
                        c.put(hash, text.clone());
                    }
                    return Some(text);
                }
            }
        }
        None
    }

    /// Cache non-empty retrieval output for `key` (failures/empties are skipped so
    /// they can be retried). Keyed by hash so the constellation can advertise/serve it.
    pub(crate) fn retrieval_put(&self, key: String, value: &str) {
        if value.is_empty() {
            return;
        }
        if let Some(c) = &self.retrieval_cache {
            c.put(crate::constellation::hash_key(&key), value.to_string());
        }
    }

    /// Fetch a URL's bytes, dodging the source when possible: the local file store
    /// first, then a constellation peer that already has it (so a cached PDF/file from arXiv,
    /// IETF, … isn't re-downloaded from the rate-limited source), then finally the
    /// source — caching the result in the store so this node and the mesh can serve
    /// it next time. With no `[store]`/`[network]` configured this is just a plain
    /// download.
    pub(crate) async fn fetch_bytes_shared(&self, url: &str) -> anyhow::Result<Vec<u8>> {
        if let Some(store) = &self.store {
            if let Some(bytes) = store.get(url).await {
                return Ok(bytes);
            }
        }
        if let Some(constellation) = self.registry.constellation() {
            if let Some(bytes) = constellation.consult_blob(url).await {
                if let Some(store) = &self.store {
                    let _ = store.put(url, &bytes).await;
                }
                return Ok(bytes);
            }
        }
        let bytes = self
            .http
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?
            .to_vec();
        if let Some(store) = &self.store {
            let _ = store.put(url, &bytes).await;
        }
        Ok(bytes)
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
            .with_instructions(include_str!("../docs/instructions.md").to_string())
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

/// Build a result cache from `[cache]`: the shared Redis store when
/// `backend = "redis"` (falling back to in-memory on connect failure), else the
/// in-memory backend. `prefix` namespaces keys so the search and retrieval caches
/// can share one Redis DB without colliding.
async fn build_cache(cfg: &config::Cache, prefix: &str) -> Arc<cache::TtlCache> {
    let ttl = cfg.ttl_secs.max(1);
    if cfg.backend.trim().eq_ignore_ascii_case("redis") {
        if cfg.redis_url.trim().is_empty() {
            tracing::warn!("[cache].backend = redis but redis_url is empty; using in-memory cache");
        } else {
            match cache::TtlCache::connect_redis(cfg.redis_url.trim(), ttl, prefix).await {
                Ok(c) => {
                    tracing::info!(prefix, "cache backend: redis");
                    return Arc::new(c);
                }
                Err(e) => tracing::warn!(
                    error = %e,
                    "redis cache unavailable; falling back to in-memory cache"
                ),
            }
        }
    }
    Arc::new(cache::TtlCache::new(ttl, cfg.max_entries))
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

/// Constellation peer endpoints (`/constellation/digest`, `/constellation/query`), each guarded by the
/// optional `[network].token`. Returns only cached search results — never secrets.
fn constellation_routes(constellation: Arc<constellation::Constellation>) -> axum::Router {
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};

    async fn digest(
        State(constellation): State<Arc<constellation::Constellation>>,
        headers: HeaderMap,
    ) -> axum::response::Response {
        if !constellation.token_ok(bearer_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        axum::Json(constellation.digest().await).into_response()
    }

    // Serve a shared file-store blob (raw bytes) by hash, or 204 if we don't have it.
    async fn blob(
        State(constellation): State<Arc<constellation::Constellation>>,
        headers: HeaderMap,
        axum::Json(req): axum::Json<constellation::BlobReq>,
    ) -> axum::response::Response {
        if !constellation.token_ok(bearer_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        match constellation.blob_lookup(&req.key).await {
            Some(bytes) => {
                constellation.record_served(&req.key, bytes.len());
                bytes.into_response()
            }
            None => StatusCode::NO_CONTENT.into_response(),
        }
    }

    // Report a blob's content hash (no bytes) so peers can corroborate it before
    // trusting any bytes — the anti-tamper handshake for shared blobs.
    async fn blobinfo(
        State(constellation): State<Arc<constellation::Constellation>>,
        headers: HeaderMap,
        axum::Json(req): axum::Json<constellation::BlobReq>,
    ) -> axum::response::Response {
        if !constellation.token_ok(bearer_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        match constellation.blob_content_hash(&req.key).await {
            Some(info) => axum::Json(info).into_response(),
            None => StatusCode::NO_CONTENT.into_response(),
        }
    }

    async fn query(
        State(constellation): State<Arc<constellation::Constellation>>,
        headers: HeaderMap,
        axum::Json(req): axum::Json<constellation::QueryReq>,
    ) -> axum::response::Response {
        if !constellation.token_ok(bearer_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        // Serve from our cache, or relay one+ hops toward a holder (bounded).
        let hits = constellation
            .answer_query(&req.key, req.ttl, &req.seen)
            .await;
        if hits.is_empty() {
            return StatusCode::NO_CONTENT.into_response();
        }
        axum::Json(constellation::QueryResp { hits }).into_response()
    }

    axum::Router::new()
        .route("/constellation/digest", get(digest))
        .route("/constellation/query", post(query))
        .route("/constellation/blob", post(blob))
        .route("/constellation/blobinfo", post(blobinfo))
        .with_state(constellation)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lodestone_mcp=info,rmcp=warn".into()),
        )
        .init();

    let mut cfg = Config::load();
    // Default the constellation node id to a stable, machine-derived id (mixed with the
    // bind port) when not set explicitly — so peers identify each other by a
    // consistent, machine-unique id across restarts rather than a random value.
    if cfg.network.enabled && cfg.network.node_id.trim().is_empty() {
        cfg.network.node_id = constellation::default_node_id(&cfg.bind);
    }
    // Wrap the resolved config in an Arc so it can be cheaply cloned into
    // Lodestone (the `features` tool needs the full config for introspection)
    // without forcing Clone derives across every sub-section. All downstream
    // uses keep working via Deref (`cfg.memory.clone()` etc. read through the
    // Arc transparently).
    let cfg = Arc::new(cfg);
    providers::configure_code_sites(cfg.code.sites.clone());
    browser::configure(browser::BrowserOptions {
        chrome_path: cfg.google.chrome_path.clone(),
        no_sandbox: cfg.google.no_sandbox,
        args: cfg.google.args.clone(),
        render_concurrency: cfg.google.render_concurrency,
    });
    // Optional on-disk file store for fetched bytes (the store_* tools). Built
    // before the constellation so the constellation can also share the store's bytes over the mesh.
    let store = if cfg.store.enabled {
        match store::FileStore::open(&cfg.store.dir, cfg.store.max_bytes, cfg.store.ttl_secs).await
        {
            Ok(s) => {
                tracing::info!(dir = %s.dir().display(), "file store enabled");
                Some(Arc::new(s))
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not open file store; store_* tools disabled");
                None
            }
        }
    } else {
        None
    };

    // The result cache is shared with the constellation (which reads/serves from it),
    // so enabling the network implies an active cache even if [cache] is off.
    let cache = if cfg.cache.enabled || cfg.network.enabled {
        Some(build_cache(&cfg.cache, "lodestone:search:").await)
    } else {
        None
    };
    // The retrieval-output cache (page/PDF/doc text). Built before the constellation so the
    // constellation can also advertise + serve it as blobs (all behind the digest Bloom).
    let retrieval_cache = if cfg.cache.enabled {
        Some(build_cache(&cfg.cache, "lodestone:ret:").await)
    } else {
        None
    };
    let constellation = cfg.network.enabled.then(|| {
        constellation::Constellation::new(
            &cfg.network,
            cache.clone().expect("cache exists when network enabled"),
            store.clone(),
            retrieval_cache.clone(),
        )
    });
    let registry = Arc::new(Registry::from_config(
        &cfg,
        cache.clone(),
        constellation.clone(),
    ));
    tracing::info!("\n{}", registry.describe());

    // Gate the local-system tool families by their config: when a family is off,
    // hide all its tools. (Destructive actions are NOT hidden — they're exposed and
    // gated at call time by the confirmation guard.) Done by extending the [tools]
    // denylist before the router is built, so the gating reuses the same filtering path.
    let tools_disabled = effective_disabled(&cfg);

    let memory = skills::memory::Memory::new(cfg.memory.clone())
        .await
        .map_err(|e| anyhow::anyhow!("failed to initialize memory store: {e:#}"))?;
    if cfg.memory.enabled {
        tracing::info!("memory store enabled dir={}", cfg.memory.dir);
        // Startup prune is opt-in: a misconfigured retention shouldn't
        // surprise-delete history on the first boot after upgrading. Verify
        // the policy with `conversation_prune dry_run=true` first, then flip
        // [memory].prune_on_startup = true.
        if cfg.memory.prune_on_startup
            && (cfg.memory.conversation_retention_days > 0 || cfg.memory.max_conversations > 0)
        {
            match memory
                .prune_conversations(
                    cfg.memory.conversation_retention_days,
                    cfg.memory.max_conversations,
                    false,
                )
                .await
            {
                Ok(n) if n > 0 => tracing::info!(
                    "startup prune: deleted {n} conversation{} \
                     (retention_days={}, max_conversations={})",
                    if n == 1 { "" } else { "s" },
                    cfg.memory.conversation_retention_days,
                    cfg.memory.max_conversations
                ),
                Ok(_) => tracing::info!("startup prune: nothing to delete"),
                Err(e) => {
                    tracing::warn!("startup prune failed: {e:#}");
                }
            }
        }
    }

    let server = Lodestone::new(
        registry,
        cfg.stackexchange.default_site.clone(),
        cfg.stackexchange.key.clone(),
        cfg.stackexchange.allowed_sites.clone(),
        cfg.github.token.clone(),
        cfg.nasa.key.clone(),
        cfg.eia.key.clone(),
        cfg.serial.clone(),
        cfg.search.timeout_secs,
        retrieval_cache,
        cfg.retrieval.default_chars,
        cfg.retrieval.max_chars,
        cfg.docker.clone(),
        cfg.kubernetes.clone(),
        cfg.filesystem.clone(),
        cfg.shell.clone(),
        cfg.git.clone(),
        cfg.databases.clone(),
        store,
        memory,
        cfg.python.clone(),
        cfg.systemd.clone(),
        cfg.clone(),
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

    // Constellation: mount peer endpoints and start discovery/sync (opt-in).
    if let Some(h) = &constellation {
        let port_of = |addr: &str| addr.rsplit(':').next().and_then(|p| p.parse::<u16>().ok());
        let sep_bind = cfg.network.bind.trim();
        if sep_bind.is_empty() {
            // Share the MCP listener: mount /constellation/* on the main app.
            let bind_port = port_of(&cfg.bind).unwrap_or(0);
            app = app.merge(constellation_routes(h.clone()));
            h.clone().start(bind_port);
            tracing::info!(
                peers = cfg.network.peers.len(),
                mdns = cfg.network.mdns,
                "constellation enabled (shares the MCP port)"
            );
        } else {
            // Separate listener: expose ONLY /constellation/* here so this port can be
            // forwarded (galaxy ingress) without publishing the MCP endpoint.
            let cbind = sep_bind.to_string();
            let advertise_port = port_of(&cbind).unwrap_or(0);
            h.clone().start(advertise_port);
            let router = constellation_routes(h.clone());
            tokio::spawn(async move {
                match tokio::net::TcpListener::bind(&cbind).await {
                    Ok(l) => {
                        tracing::info!("constellation listening on http://{cbind}/constellation");
                        if let Err(e) = axum::serve(l, router).await {
                            tracing::error!(error = %e, "constellation listener stopped");
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, bind = %cbind, "constellation bind failed")
                    }
                }
            });
            tracing::info!(
                peers = cfg.network.peers.len(),
                mdns = cfg.network.mdns,
                "constellation enabled (separate port; MCP not exposed here)"
            );
        }
    }

    // Galaxy participation (the broker itself is a SEPARATE binary, `lodestone-galaxy`):
    // register this constellation with the configured brokers and pull their
    // directories, adding other constellations' ingress endpoints as peers (so
    // consults reach them directly). A node joins its own constellation first
    // (warm-up) before reaching out.
    if let Some(h) = &constellation {
        if !cfg.galaxy.servers.is_empty() {
            // Empty id → the client registers under the shared constellation id.
            let ghttp = reqwest::Client::builder()
                .user_agent("lodestone-galaxy")
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            galaxy::client::GalaxyClient {
                http: ghttp,
                servers: cfg.galaxy.servers.clone(),
                id: cfg.galaxy.id.clone(),
                ingress: cfg.galaxy.ingress.clone(),
                token: cfg.galaxy.token.clone(),
                heartbeat_secs: cfg.galaxy.heartbeat_secs,
                join_warmup_secs: cfg.galaxy.join_warmup_secs,
            }
            .start(h.clone());
            tracing::info!(
                servers = cfg.galaxy.servers.len(),
                ingress = cfg.galaxy.ingress.len(),
                "galaxy participation enabled"
            );
        }
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
