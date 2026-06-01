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
mod retrieval;
mod skills;
mod store;
mod tracing_control;
mod util;
mod ws;

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

/// The "honest" User-Agent we send to APIs that require explicit attribution
/// (Overpass, callook, GitHub, Wikipedia, FCC, etc.) and that every live
/// integration test reuses. Centralized so a single version bump propagates
/// everywhere — chasing it down across 30+ files was the original wet-code
/// motivation.
pub(crate) const LODESTONE_UA: &str =
    "lodestone-mcp/0.1.0 (+https://github.com/elyerinfox/lodestone-mcp)";

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
    /// Caches retrieval-tool output (page text, files, answers) keyed by request
    /// and by the entry's [`crate::constellation::Identifiers`] aliases (URL,
    /// source-id, content hash). The constellation digest advertises every
    /// identifier hash so a peer that asks by *any* of them gets a Bloom hit.
    pub(crate) retrieval_cache: Option<Arc<retrieval::IndexedRetrievalCache>>,
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
    /// Server boot time. Captured at `Lodestone::new`; consumed by the
    /// `/ws/status` dashboard feed to compute uptime without dragging the
    /// server's local clock into the wire format.
    pub(crate) started_at: std::time::Instant,
    /// The set of tool names the resolved config has gated off. Precomputed
    /// at startup (the source of truth used to build the tool router) so the
    /// `features` skill can map families to "any of these tools hidden?"
    /// without re-running the resolution.
    pub(crate) disabled_tools: Arc<Vec<String>>,
    /// Tools the dashboard's settings drawer has flipped off at runtime.
    /// Empty at startup; mutated by `POST /api/settings/tools`. The
    /// dispatch wrapper (`skills::route`) checks this set before
    /// running each call and returns a "tool disabled" error if hit.
    /// Ephemeral — never persisted, so a restart restores the resolved
    /// active set from config.
    pub(crate) runtime_disabled_tools: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    /// Per-family capability probe results, cached at startup. The
    /// dispatch wrapper consults this to refuse calls whose family is
    /// `Unavailable` with a clean error (carrying the probe's reason +
    /// hint so the LLM sees what's missing). The WS snapshot exposes it
    /// to the dashboard. Pure-Rust families that didn't register a
    /// `FamilyMeta` impl don't appear here and dispatch without a gate
    /// — same behavior as before this feature shipped.
    pub(crate) skill_capabilities:
        Arc<std::collections::HashMap<&'static str, skills::SkillCapability>>,
    /// `tool_name → family_name` reverse index built alongside
    /// `skill_capabilities` so the dispatch wrapper can look up the
    /// owning family in O(1).
    pub(crate) tool_to_family: Arc<std::collections::HashMap<&'static str, &'static str>>,
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
        retrieval_cache: Option<Arc<retrieval::IndexedRetrievalCache>>,
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
            runtime_disabled_tools: Arc::new(std::sync::Mutex::new(
                std::collections::HashSet::new(),
            )),
            skill_capabilities: {
                let mut map: std::collections::HashMap<&'static str, skills::SkillCapability> =
                    std::collections::HashMap::new();
                for fam in skills::families() {
                    let cap = fam.check_capability();
                    if let skills::SkillCapability::Unavailable { reason, hint } = &cap {
                        match hint {
                            Some(h) => tracing::warn!(
                                family = fam.family(),
                                reason = %reason,
                                hint = %h,
                                "skill family unavailable on this host — \
                                 calls into it will be refused at dispatch"
                            ),
                            None => tracing::warn!(
                                family = fam.family(),
                                reason = %reason,
                                "skill family unavailable on this host"
                            ),
                        }
                    }
                    map.insert(fam.family(), cap);
                }
                Arc::new(map)
            },
            tool_to_family: {
                let mut map: std::collections::HashMap<&'static str, &'static str> =
                    std::collections::HashMap::new();
                for fam in skills::families() {
                    let name = fam.family();
                    for t in fam.tools() {
                        map.insert(*t, name);
                    }
                }
                Arc::new(map)
            },
            cfg,
            started_at: std::time::Instant::now(),
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
    /// letting one node's fetched/parsed text serve the mesh.
    ///
    /// Single-key shim — equivalent to calling [`Self::retrieval_lookup`] with
    /// `Identifiers::new(key)`. New call sites that have a URL / source-id
    /// available should call `retrieval_lookup` directly so a peer that
    /// cached under a different canonical key can still serve us.
    pub(crate) async fn retrieval_get(&self, key: &str) -> Option<String> {
        self.retrieval_lookup(&crate::constellation::Identifiers::new(key))
            .await
    }

    /// Look up an entry by **any** of its identifiers (primary key, URL,
    /// source-id, content hash). Walks the local cache first — each
    /// identifier is hashed and probed — then, on a true miss, asks
    /// Bloom-matching constellation peers by each identifier hash in turn.
    /// First hit wins.
    ///
    /// The multi-identifier path is what closes the alignment gap on
    /// long-tail rate-limited content: a consumer asking by URL finds an
    /// entry a peer cached by source-id (and vice versa).
    pub(crate) async fn retrieval_lookup(
        &self,
        ids: &crate::constellation::Identifiers,
    ) -> Option<String> {
        if let Some(c) = &self.retrieval_cache {
            if let Some(v) = c.lookup(ids) {
                return Some(v);
            }
        }
        if let Some(constellation) = self.registry.constellation() {
            for r in ids.iter_capped() {
                let h = crate::constellation::hash_key(&r.key());
                // Pass the source hint so content-addressable upstreams
                // (Wayback / arXiv / GitHub) skip the multi-peer corroboration
                // floor — a single peer suffices because the consumer's
                // bytes-hash check is the primary safety.
                if let Some(bytes) = constellation
                    .consult_blob_hash_sourced(&h, ids.source)
                    .await
                {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    if !text.is_empty() {
                        if let Some(c) = &self.retrieval_cache {
                            c.put(ids, &text);
                        }
                        return Some(text);
                    }
                }
            }
        }
        None
    }

    /// Cache non-empty retrieval output under one primary key (no aliases,
    /// `Source::Other`). Bare-key shim for existing call sites; new code
    /// should call [`Self::retrieval_put_indexed`] with an `Identifiers`
    /// carrying every public name the entry is known by, so peer
    /// alignment works for long-tail content.
    pub(crate) fn retrieval_put(&self, key: String, value: &str) {
        if value.is_empty() {
            return;
        }
        self.retrieval_put_indexed(&crate::constellation::Identifiers::new(key), value);
    }

    /// Cache non-empty retrieval output with a full `Identifiers` set
    /// (primary key + URL aliases + source-id aliases). Every identifier is
    /// hashed and stored in the secondary index; the constellation digest
    /// will advertise all of them on its next sync so peers can find the
    /// entry by any of its public names.
    pub(crate) fn retrieval_put_indexed(
        &self,
        ids: &crate::constellation::Identifiers,
        value: &str,
    ) {
        if value.is_empty() {
            return;
        }
        if let Some(c) = &self.retrieval_cache {
            c.put(ids, value);
        }
    }

    /// Fetch a URL's bytes, **lookup order**:
    /// 1. **Local file store** — already-downloaded bytes.
    /// 2. **Peer cache** (via constellation `consult_blob`) — a peer that
    ///    has it served the bytes without anyone re-hitting upstream.
    /// 3. **Direct upstream fetch** — try plain HTTP first; on failure
    ///    (which typically means rate-limit / 429), fall through to step 4.
    /// 4. **Peer-delegated fetch** — ask a peer that advertised
    ///    `delegation_enabled = true` to fetch it for us, subject to that
    ///    peer's per-hour rate limits. The serving peer caches the result
    ///    too, so the mesh now has it cached behind the Bloom for everyone.
    ///
    /// Steps 2 and 4 require `[network].enabled` and a peer in the
    /// constellation. Step 4 additionally requires at least one peer with
    /// `delegation_enabled = true`. The path collapses gracefully — with
    /// none of those configured this is just a plain HTTP download.
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
        // Direct upstream fetch.
        let direct = self
            .http
            .get(url)
            .send()
            .await
            .and_then(|r| r.error_for_status());
        match direct {
            Ok(resp) => {
                let bytes = resp.bytes().await?.to_vec();
                if let Some(store) = &self.store {
                    let _ = store.put(url, &bytes).await;
                }
                Ok(bytes)
            }
            Err(direct_err) => {
                // Direct fetch failed — typically rate-limited (429) or
                // geo-blocked. Try delegating to a willing peer.
                if let Some(constellation) = self.registry.constellation() {
                    if let Some(bytes) = constellation
                        .delegated_fetch(
                            url,
                            self.max_chars as u64 * 4, // a chars-budget cap converts roughly to bytes
                            crate::constellation::Source::Other,
                        )
                        .await
                    {
                        if let Some(store) = &self.store {
                            let _ = store.put(url, &bytes).await;
                        }
                        tracing::info!(
                            url,
                            "fetch_bytes_shared: served via peer delegation after direct fetch failed"
                        );
                        return Ok(bytes);
                    }
                }
                Err(anyhow::anyhow!(direct_err))
            }
        }
    }

    /// Build a privacy-safe snapshot of server / memory / constellation
    /// state for the `/ws/status` dashboard feed. Counts only — no row
    /// bodies, no secrets, no peer auth material. The frontend renders
    /// the dashboard from this; nothing else flows through this channel.
    pub(crate) async fn ws_snapshot(&self) -> crate::ws::Snapshot {
        // Server.
        let providers: Vec<crate::ws::ProviderEntry> = self
            .registry
            .list()
            .into_iter()
            .map(|(kind, id)| crate::ws::ProviderEntry {
                kind: format!("{kind:?}").to_lowercase(),
                id: id.to_string(),
            })
            .collect();
        // Partition the static tool name list into active vs config-gated
        // sets. Sorting once here lets the frontend skip a re-sort and
        // diff cleanly when the snapshot refreshes.
        let mut tools_active_names: Vec<String> = skills::registered_tool_names()
            .into_iter()
            .filter(|n| !self.disabled_tools.iter().any(|d| d == n))
            .collect();
        tools_active_names.sort();
        let mut tools_disabled_names: Vec<String> = (*self.disabled_tools).clone();
        tools_disabled_names.sort();
        let mut tools_runtime_disabled_names: Vec<String> = self
            .runtime_disabled_tools
            .lock()
            .unwrap()
            .iter()
            .cloned()
            .collect();
        tools_runtime_disabled_names.sort();
        let server = crate::ws::ServerStatus {
            name: "lodestone-mcp",
            version: env!("CARGO_PKG_VERSION"),
            uptime_secs: self.started_at.elapsed().as_secs(),
            tools_active: tools_active_names.len(),
            tools_disabled: tools_disabled_names.len(),
            tools_active_names,
            tools_disabled_names,
            tools_runtime_disabled_names,
            providers,
            bind: self.cfg.bind.clone(),
            constellation_bind: self.cfg.network.bind.clone(),
            secrets: crate::ws::SecretPresence {
                auth_token: !self.cfg.auth_token.trim().is_empty(),
                network_token: !self.cfg.network.token.trim().is_empty(),
                github_token: !self.github_token.trim().is_empty(),
                nasa_key: !self.nasa_key.trim().is_empty(),
                eia_key: !self.eia_key.trim().is_empty(),
            },
            log_level: tracing_control::current(),
            skill_capabilities: {
                // Rebuild the family→tools map fresh so the row order is
                // deterministic. Capability results come from the cache
                // populated at startup. Pure-Rust families that
                // didn't register a FamilyMeta impl are absent — the
                // dashboard renders the existing flat list for those.
                let mut rows: Vec<crate::ws::SkillCapabilityEntry> = Vec::new();
                for fam in skills::families() {
                    let cap = self.skill_capabilities.get(fam.family());
                    let (ready, reason, hint) = match cap {
                        Some(skills::SkillCapability::Ready) | None => (true, None, None),
                        Some(skills::SkillCapability::Unavailable { reason, hint }) => {
                            (false, Some(reason.clone()), hint.clone())
                        }
                    };
                    rows.push(crate::ws::SkillCapabilityEntry {
                        family: fam.family().to_string(),
                        tools: fam.tools().iter().map(|s| s.to_string()).collect(),
                        ready,
                        reason,
                        hint,
                    });
                }
                rows.sort_by(|a, b| a.family.cmp(&b.family));
                rows
            },
        };
        // Memory. Internal struct uses i64 (SQLite native); convert to u64
        // for the wire format (negatives can't happen — these are
        // COUNT(*) results).
        let mem_stats = self.memory.stats().await;
        let memory_enabled = self.memory.enabled();
        let memory = crate::ws::MemoryStats {
            enabled: memory_enabled,
            memos: mem_stats.memos.max(0) as u64,
            solutions: mem_stats.solutions.max(0) as u64,
            solution_revisions: mem_stats.solution_revisions.max(0) as u64,
            solution_tags: mem_stats.solution_tags.max(0) as u64,
            solution_links: mem_stats.solution_links.max(0) as u64,
            solution_phrasings: mem_stats.solution_phrasings.max(0) as u64,
            conversations: mem_stats.conversations.max(0) as u64,
            conversation_turns: mem_stats.conversation_turns.max(0) as u64,
            synonyms: mem_stats.synonyms.max(0) as u64,
            db_path: if memory_enabled {
                self.cfg.memory.dir.clone()
            } else {
                String::new()
            },
            embedding_model: if memory_enabled {
                self.cfg.memory.embedding_model.clone()
            } else {
                String::new()
            },
            auto_recall: self.memory.auto_recall_enabled(),
            record_conversations: self.memory.record_conversations_enabled(),
        };
        // Constellation. When the network is off, return a sensible
        // "disabled" snapshot so the frontend can show "constellation
        // disabled" without trying to parse missing fields.
        let constellation = match self.registry.constellation() {
            Some(c) => c.ws_state(),
            None => crate::ws::ConstellationState::default(),
        };
        // Browser sessions are lazily initialized: the manager exists
        // (`browser_open` would have created it on first call) only if
        // some tool has touched it. Skip the OnceCell init here so the
        // snapshot is cheap when the model never used the browser.
        let browser = match crate::skills::browser_session::manager_if_init() {
            Some(mgr) => {
                let cfg = mgr.config().await;
                crate::ws::BrowserState {
                    sessions: mgr.list_live().await,
                    personas: mgr.persona_list().await,
                    guest_sessions: mgr.guest_session_list().await,
                    idle_timeout_secs: cfg.idle_timeout_secs,
                    max_concurrent: cfg.max_concurrent,
                }
            }
            None => crate::ws::BrowserState::default(),
        };
        crate::ws::Snapshot {
            server,
            memory,
            constellation,
            browser,
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

    /// `POST /constellation/retrieve` — the "go fetch this URL for me"
    /// delegation endpoint. Gated by `[network].token` and
    /// `[network].delegation_enabled`. The requester identifies itself via
    /// `X-Lodestone-Peer-Id` (its node id) so the sliding-window rate
    /// limiter can account per-peer; the cluster token already gates who
    /// can ask in the first place, so spoofing the peer-id only burns the
    /// requester's own quota faster than necessary.
    async fn retrieve(
        State(constellation): State<Arc<constellation::Constellation>>,
        headers: HeaderMap,
        axum::Json(req): axum::Json<constellation::RetrieveReq>,
    ) -> axum::response::Response {
        if !constellation.token_ok(bearer_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        let peer_id = headers
            .get("x-lodestone-peer-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        match constellation.serve_retrieve(&peer_id, &req).await {
            Ok(bytes) => bytes.into_response(),
            Err(reject) => {
                // `disabled` and `per_job_too_large` aren't retryable; the
                // others carry a Retry-After hint. Map both to the standard
                // HTTP semantics so clients without machine-readable
                // handling still do something sensible.
                let status = match reject.reason {
                    "disabled" => StatusCode::FORBIDDEN,
                    "per_job_too_large" => StatusCode::PAYLOAD_TOO_LARGE,
                    "peer_jobs_exceeded" | "global_bytes_exceeded" => StatusCode::TOO_MANY_REQUESTS,
                    _ => StatusCode::BAD_GATEWAY,
                };
                let mut resp = (status, axum::Json(&reject)).into_response();
                if reject.retry_after_secs > 0 {
                    if let Ok(val) = reject.retry_after_secs.to_string().parse() {
                        resp.headers_mut().insert("Retry-After", val);
                    }
                }
                resp
            }
        }
    }

    /// `POST /constellation/browser_persona` — the "drive your browser
    /// session for me" delegation endpoint (#128). Gated by
    /// `[network].token` and `[network.capabilities].browser`.
    /// Sessions do NOT transport; each node uses its OWN persona. The
    /// peer's SSRF guard refuses any URL that resolves to its local
    /// network so a delegated request can't be a LAN-enumeration vector.
    async fn browser_persona(
        State(constellation): State<Arc<constellation::Constellation>>,
        headers: HeaderMap,
        axum::Json(req): axum::Json<constellation::BrowserPersonaReq>,
    ) -> axum::response::Response {
        if !constellation.token_ok(bearer_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        // The cluster token already gates *who* can ask; the peer id
        // header is for per-peer persona ISOLATION — peer A's "google"
        // and peer B's "google" become separate browser contexts.
        // A spoofed id only buys the requester someone else's
        // cookies on their own logical persona name, never a leak across
        // legitimate peers (each gets `delegated:<their-id>:<name>`).
        let peer_id = headers
            .get("x-lodestone-peer-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        match constellation.answer_browser_persona(&peer_id, &req).await {
            Ok(body) => axum::Json(body).into_response(),
            Err(reject) => {
                let status = match reject.reason {
                    "disabled" => StatusCode::FORBIDDEN,
                    "navigate_failed" => StatusCode::BAD_GATEWAY,
                    "persona_unavailable" => StatusCode::SERVICE_UNAVAILABLE,
                    _ => StatusCode::BAD_REQUEST,
                };
                (status, axum::Json(&reject)).into_response()
            }
        }
    }

    axum::Router::new()
        .route("/constellation/digest", get(digest))
        .route("/constellation/query", post(query))
        .route("/constellation/blob", post(blob))
        .route("/constellation/blobinfo", post(blobinfo))
        .route("/constellation/retrieve", post(retrieve))
        .route("/constellation/browser_persona", post(browser_persona))
        .with_state(constellation)
}

/// `/ws/status` — the dashboard WebSocket feed. One-way push: snapshot on
/// connect, then a fresh snapshot every [`ws::PUSH_INTERVAL`]. Auth via
/// the `[network].token` (passed as `?token=…` since the browser's
/// `WebSocket` constructor can't set custom headers); open when no token
/// is configured. See `src/ws.rs` for the message envelope.
fn ws_routes(server: Arc<Lodestone>) -> axum::Router {
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::extract::{Query, State};
    use axum::response::IntoResponse;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct AuthQuery {
        #[serde(default)]
        token: String,
    }

    async fn handler(
        ws: WebSocketUpgrade,
        State(server): State<Arc<Lodestone>>,
        Query(auth): Query<AuthQuery>,
    ) -> axum::response::Response {
        let configured = server.cfg.network.token.trim();
        if !configured.is_empty()
            && !util::ct_eq(auth.token.trim().as_bytes(), configured.as_bytes())
        {
            return (axum::http::StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        ws.on_upgrade(|sock| run(server, sock))
    }

    async fn run(server: Arc<Lodestone>, mut sock: WebSocket) {
        // Send the initial snapshot immediately so the dashboard renders
        // on connect, then loop pushing one every PUSH_INTERVAL until the
        // client disconnects.
        let mut tick = tokio::time::interval(ws::PUSH_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let snap = server.ws_snapshot().await;
            let msg = ws::WsMessage::Snapshot(snap);
            match serde_json::to_string(&msg) {
                Ok(payload) => {
                    if sock.send(Message::Text(payload.into())).await.is_err() {
                        return; // client gone
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "ws snapshot serialize failed");
                }
            }
            // Wait for next tick OR a client message (we ignore inbound
            // text for v1, but we DO need to drive the socket to detect
            // clean closes).
            tokio::select! {
                _ = tick.tick() => {}
                msg = sock.recv() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | None => return,
                        Some(Err(_)) => return,
                        _ => {} // ignore other inbound; v1 is push-only
                    }
                }
            }
        }
    }

    axum::Router::new()
        .route("/ws/status", axum::routing::get(handler))
        .with_state(server)
}

/// `/api/settings/*` — ephemeral, per-subsystem runtime tuners that the
/// dashboard's settings drawers POST to. Authenticated against the same
/// `[network].token` as the WebSocket feed (constant-time compare).
/// Changes apply to the running process only and are NOT persisted to
/// disk, so a restart restores the config file's values. Knobs that
/// require subsystem lifecycle changes (mDNS daemon, sync interval)
/// are intentionally absent — see `ConstellationState.*_configured`.
/// Secrets are never accepted here: the network token, auth token, and
/// any future API keys can only be set via config or env.
fn api_routes(
    server: Arc<Lodestone>,
    constellation: Arc<constellation::Constellation>,
) -> axum::Router {
    use axum::extract::State;
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::Json;

    /// Bundle the two handles a settings endpoint might need.
    #[derive(Clone)]
    struct ApiState {
        #[allow(dead_code)] // used by handlers added in follow-up tasks
        server: Arc<Lodestone>,
        constellation: Arc<constellation::Constellation>,
    }

    fn presented_token(headers: &HeaderMap) -> Option<&str> {
        let auth = headers.get(axum::http::header::AUTHORIZATION)?;
        let s = auth.to_str().ok()?;
        Some(s.strip_prefix("Bearer ").unwrap_or(s).trim())
    }

    async fn patch_constellation(
        State(state): State<ApiState>,
        headers: HeaderMap,
        Json(patch): Json<constellation::RuntimeOverridesPatch>,
    ) -> axum::response::Response {
        if !state.constellation.token_ok(presented_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        let applied = state.constellation.apply_runtime_patch(patch);
        Json(serde_json::json!({
            "delegation_enabled": applied.delegation_enabled,
            "max_peers": applied.max_peers,
            "min_agreement": applied.min_agreement,
        }))
        .into_response()
    }

    #[derive(serde::Deserialize)]
    struct ServerPatch {
        log_level: Option<String>,
    }

    async fn patch_server(
        State(state): State<ApiState>,
        headers: HeaderMap,
        Json(patch): Json<ServerPatch>,
    ) -> axum::response::Response {
        if !state.constellation.token_ok(presented_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        if let Some(lvl) = patch.log_level.as_deref() {
            if let Err(e) = tracing_control::set_level(lvl) {
                return (StatusCode::BAD_REQUEST, format!("{e}\n")).into_response();
            }
        }
        Json(serde_json::json!({
            "log_level": tracing_control::current(),
        }))
        .into_response()
    }

    #[derive(serde::Deserialize)]
    struct MemoryPatch {
        enabled: Option<bool>,
        auto_recall: Option<bool>,
        record_conversations: Option<bool>,
    }

    async fn patch_memory(
        State(state): State<ApiState>,
        headers: HeaderMap,
        Json(patch): Json<MemoryPatch>,
    ) -> axum::response::Response {
        if !state.constellation.token_ok(presented_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        state
            .server
            .memory
            .apply_runtime_patch(crate::skills::memory::RuntimeOverrides {
                enabled: patch.enabled,
                auto_recall: patch.auto_recall,
                record_conversations: patch.record_conversations,
            });
        Json(serde_json::json!({
            "enabled": state.server.memory.enabled(),
            "auto_recall": state.server.memory.auto_recall_enabled(),
            "record_conversations": state.server.memory.record_conversations_enabled(),
        }))
        .into_response()
    }

    /// `{ disabled: { "<tool_name>": true|false, ... } }` — sparse map
    /// of toggles. `true` adds the tool to the runtime-disabled set,
    /// `false` removes it. Names not in the map keep their current
    /// state. Names that aren't real tools are silently ignored
    /// (the dashboard never sends them, and accepting them would just
    /// waste memory).
    #[derive(serde::Deserialize)]
    struct ToolsPatch {
        #[serde(default)]
        disabled: std::collections::HashMap<String, bool>,
    }

    async fn patch_tools(
        State(state): State<ApiState>,
        headers: HeaderMap,
        Json(patch): Json<ToolsPatch>,
    ) -> axum::response::Response {
        if !state.constellation.token_ok(presented_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        let known: std::collections::HashSet<String> =
            crate::skills::registered_tool_names().into_iter().collect();
        let mut set = state.server.runtime_disabled_tools.lock().unwrap();
        for (name, disabled) in patch.disabled {
            if !known.contains(&name) {
                continue;
            }
            if disabled {
                set.insert(name);
            } else {
                set.remove(&name);
            }
        }
        let mut current: Vec<String> = set.iter().cloned().collect();
        current.sort();
        Json(serde_json::json!({ "disabled": current })).into_response()
    }

    async fn patch_browser(
        State(state): State<ApiState>,
        headers: HeaderMap,
        Json(patch): Json<crate::skills::browser_session::BrowserConfigPatch>,
    ) -> axum::response::Response {
        if !state.constellation.token_ok(presented_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        let mgr = crate::skills::browser_session::manager().await;
        let cfg = mgr.apply_runtime_patch(patch).await;
        Json(serde_json::json!({
            "idle_timeout_secs": cfg.idle_timeout_secs,
            "max_concurrent": cfg.max_concurrent,
        }))
        .into_response()
    }

    /// `DELETE /api/browser/sessions/:id` — kill a session from the
    /// dashboard. Idempotent at the listing level (unknown session
    /// returns 404; the dashboard refreshes from the next WS tick).
    async fn close_browser_session(
        State(state): State<ApiState>,
        headers: HeaderMap,
        axum::extract::Path(id): axum::extract::Path<String>,
    ) -> axum::response::Response {
        if !state.constellation.token_ok(presented_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        let mgr = match crate::skills::browser_session::manager_if_init() {
            Some(m) => m,
            None => return (StatusCode::NOT_FOUND, "no browser sessions\n").into_response(),
        };
        match mgr.close(&id).await {
            Ok(()) => Json(serde_json::json!({ "closed": id })).into_response(),
            Err(e) => (StatusCode::NOT_FOUND, format!("{e}\n")).into_response(),
        }
    }

    /// `POST /api/browser/personas/:name/reset` — confirm-reset a poisoned
    /// persona from the dashboard. Disposes the current session+context
    /// and creates a fresh one; the persona state returns to healthy.
    async fn reset_browser_persona(
        State(state): State<ApiState>,
        headers: HeaderMap,
        axum::extract::Path(name): axum::extract::Path<String>,
    ) -> axum::response::Response {
        if !state.constellation.token_ok(presented_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        let mgr = match crate::skills::browser_session::manager_if_init() {
            Some(m) => m,
            None => return (StatusCode::NOT_FOUND, "no browser sessions\n").into_response(),
        };
        match mgr.persona_reset(&name).await {
            Ok(sid) => Json(serde_json::json!({
                "name": name,
                "session_id": sid,
                "state": "healthy",
            }))
            .into_response(),
            Err(e) => (StatusCode::BAD_REQUEST, format!("{e}\n")).into_response(),
        }
    }

    /// `GET /api/memory/graph` — solution graph snapshot for the
    /// dashboard explorer. Query params:
    /// - `mode`: `all` (default) | `filter` | `focus`
    /// - `tag`, `query`, `hide_superseded` (for `filter` mode)
    /// - `id`, `depth` (for `focus` mode)
    ///
    /// Returns `{ nodes: [...], edges: [...] }`. Auth: same bearer
    /// as the WS feed and other /api/* endpoints.
    #[derive(serde::Deserialize)]
    struct GraphQuery {
        #[serde(default)]
        mode: Option<String>,
        #[serde(default)]
        tag: Option<String>,
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        hide_superseded: Option<bool>,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        depth: Option<u32>,
    }
    async fn memory_graph(
        State(state): State<ApiState>,
        headers: HeaderMap,
        axum::extract::Query(q): axum::extract::Query<GraphQuery>,
    ) -> axum::response::Response {
        if !state.constellation.token_ok(presented_token(&headers)) {
            return (StatusCode::UNAUTHORIZED, "unauthorized\n").into_response();
        }
        let mode = match q.mode.as_deref() {
            Some("filter") => crate::skills::memory::GraphMode::Filter {
                tag: q.tag,
                query: q.query,
                hide_superseded: q.hide_superseded.unwrap_or(false),
            },
            Some("focus") => match q.id {
                Some(id) if !id.trim().is_empty() => crate::skills::memory::GraphMode::Focus {
                    id,
                    depth: q.depth.unwrap_or(2),
                },
                _ => return (StatusCode::BAD_REQUEST, "focus mode requires id\n").into_response(),
            },
            _ => crate::skills::memory::GraphMode::All,
        };
        let snap = state.server.memory.graph_snapshot(mode).await;
        Json(snap).into_response()
    }

    let state = ApiState {
        server,
        constellation,
    };
    axum::Router::new()
        .route(
            "/api/settings/constellation",
            axum::routing::post(patch_constellation),
        )
        .route("/api/settings/server", axum::routing::post(patch_server))
        .route("/api/settings/memory", axum::routing::post(patch_memory))
        .route("/api/settings/tools", axum::routing::post(patch_tools))
        .route("/api/settings/browser", axum::routing::post(patch_browser))
        .route(
            "/api/browser/sessions/{id}",
            axum::routing::delete(close_browser_session),
        )
        .route(
            "/api/browser/personas/{name}/reset",
            axum::routing::post(reset_browser_persona),
        )
        .route("/api/memory/graph", axum::routing::get(memory_graph))
        .with_state(state)
        // The dashboard SPA may be served from a different origin
        // (port-separated standalone container, remote deployment).
        // The /api/* surface is already auth-gated by the bearer
        // token, so the CORS layer just adds permissive headers + a
        // 204 reply for OPTIONS preflights. Embedded same-origin
        // requests get the same headers without ill effect.
        .layer(axum::middleware::from_fn(api_cors))
}

/// CORS middleware for the /api/* surface. Echoes the request's
/// `Origin`, allows GET/POST/DELETE + the headers the dashboard sends
/// (Content-Type, Authorization), and short-circuits OPTIONS
/// preflights with 204 so a cross-origin fetch from the standalone
/// dashboard container actually completes.
async fn api_cors(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{header, HeaderValue, Method, StatusCode};
    use axum::response::IntoResponse;
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .cloned()
        .unwrap_or_else(|| HeaderValue::from_static("*"));
    if req.method() == Method::OPTIONS {
        let mut resp = StatusCode::NO_CONTENT.into_response();
        let h = resp.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        h.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
        );
        h.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("authorization, content-type"),
        );
        h.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("600"),
        );
        return resp;
    }
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    h.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("content-type"),
    );
    resp
}

/// Static dashboard route — serves the Nuxt SPA embedded into the binary
/// at compile time by `build.rs` (see [`ws::DASHBOARD`]). Path layout:
/// - `GET /` → redirect to `/dashboard/`.
/// - `GET /dashboard/` → `index.html`.
/// - `GET /dashboard/{*path}` → the matching file under
///   `frontend/.output/public/`.
///
/// When the dashboard wasn't built (no npm at compile time), the route
/// returns a small HTML page telling the operator how to build it. The
/// rest of the server (MCP, `/ws/status`, constellation endpoints)
/// works regardless.
fn dashboard_routes() -> axum::Router {
    use axum::http::{header, StatusCode};
    use axum::response::{Html, IntoResponse, Redirect};

    async fn redirect_root() -> impl axum::response::IntoResponse {
        Redirect::permanent("/dashboard/")
    }

    async fn serve(path: axum::extract::Path<String>) -> axum::response::Response {
        serve_path(&path.0).await
    }

    async fn serve_index() -> axum::response::Response {
        serve_path("index.html").await
    }

    async fn serve_path(raw: &str) -> axum::response::Response {
        let path = if raw.is_empty() || raw.ends_with('/') {
            format!("{raw}index.html")
        } else {
            raw.to_string()
        };
        if let Some(file) = ws::DASHBOARD.get_file(&path) {
            let mime = mime_for(&path);
            return ([(header::CONTENT_TYPE, mime)], file.contents()).into_response();
        }
        // No file at that path. If the dashboard wasn't built at all,
        // show the "how to build" page; otherwise fall back to the
        // SPA's index.html so client-side routing still works.
        if ws::DASHBOARD.files().next().is_none() {
            return Html(NOT_BUILT_PAGE).into_response();
        }
        if let Some(index) = ws::DASHBOARD.get_file("index.html") {
            return (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                index.contents(),
            )
                .into_response();
        }
        (StatusCode::NOT_FOUND, "not found\n").into_response()
    }

    fn mime_for(path: &str) -> &'static str {
        let lower = path.to_lowercase();
        if lower.ends_with(".html") {
            "text/html; charset=utf-8"
        } else if lower.ends_with(".js") || lower.ends_with(".mjs") {
            "application/javascript"
        } else if lower.ends_with(".css") {
            "text/css; charset=utf-8"
        } else if lower.ends_with(".json") {
            "application/json"
        } else if lower.ends_with(".svg") {
            "image/svg+xml"
        } else if lower.ends_with(".png") {
            "image/png"
        } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
            "image/jpeg"
        } else if lower.ends_with(".gif") {
            "image/gif"
        } else if lower.ends_with(".webp") {
            "image/webp"
        } else if lower.ends_with(".ico") {
            "image/x-icon"
        } else if lower.ends_with(".woff2") {
            "font/woff2"
        } else if lower.ends_with(".woff") {
            "font/woff"
        } else if lower.ends_with(".map") {
            "application/json"
        } else {
            "application/octet-stream"
        }
    }

    const NOT_BUILT_PAGE: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <title>lodestone-mcp dashboard — not built</title>
  <style>
    body { font-family: ui-monospace, Menlo, Consolas, monospace; background:#0f1115; color:#e2e8f0; max-width:780px; margin:6rem auto; padding:0 1.5rem; line-height:1.6; }
    h1 { font-size:1.25rem; }
    code, pre { background:#1d2230; border:1px solid #252b3c; border-radius:6px; padding:.15rem .35rem; }
    pre { padding:1rem; overflow:auto; }
    a { color:#60a5fa; }
  </style>
</head>
<body>
  <h1>Dashboard not built</h1>
  <p>
    The <code>lodestone-mcp</code> binary was built without the Nuxt
    dashboard — usually because <code>npm</code> wasn't on <code>PATH</code>
    at compile time. Install <a href="https://nodejs.org/">Node.js</a>
    (≥ 18) and rebuild:
  </p>
  <pre>cargo clean &amp;&amp; cargo build</pre>
  <p>
    The MCP server, the <code>/ws/status</code> WebSocket feed, and the
    <code>/constellation/*</code> endpoints all work without the
    dashboard.
  </p>
  <p>
    During dashboard development you can also run Nuxt's hot-reloading
    dev server separately — see
    <code>frontend/README.md</code>.
  </p>
</body>
</html>
"#;

    axum::Router::new()
        .route("/", axum::routing::get(redirect_root))
        .route("/dashboard", axum::routing::get(serve_index))
        .route("/dashboard/", axum::routing::get(serve_index))
        .route("/dashboard/{*path}", axum::routing::get(serve))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_control::init();

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
    // Multi-identifier indexed: every entry advertises its primary key, URL aliases,
    // source-specific ids (arXiv id, Wayback `(url, timestamp)`, …) and content hash, so a
    // peer that asks by any of those names gets a Bloom hit.
    let retrieval_cache = if cfg.cache.enabled {
        Some(Arc::new(retrieval::IndexedRetrievalCache::new(
            cfg.cache.ttl_secs.max(1),
            cfg.cache.max_entries,
            // The body-byte cap doubles as the size guardrail for the
            // delegation feature — operators that opt into delegation
            // typically want to bound how much delegated traffic can
            // bloat the cache. 0 = unlimited.
            cfg.network.delegation_max_cache_bytes,
        )))
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

    // Hold an Arc handle for the WebSocket dashboard feed BEFORE the MCP
    // service closure moves `server`. The Lodestone struct's heavy fields
    // are already `Arc`-shared internally, so cloning here is cheap.
    let server_for_ws = Arc::new(server.clone());

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
        .merge(mcp)
        // `/ws/status` — dashboard push feed. Auth via `?token=…` against
        // `[network].token` (separate from `auth_token`, same trust domain
        // as the constellation endpoints).
        .merge(ws_routes(server_for_ws.clone()))
        // `/dashboard/{*path}` + `/` redirect — Nuxt SPA embedded into the
        // binary at compile time. When npm wasn't on PATH at build time
        // the route returns a "not built — install Node and rebuild"
        // page instead of the SPA.
        .merge(dashboard_routes());

    // Constellation: mount peer endpoints and start discovery/sync (opt-in).
    if let Some(h) = &constellation {
        // Per-subsystem ephemeral settings endpoints live on the MCP
        // listener (the dashboard talks to them from the same origin
        // it loads from). The constellation-port listener intentionally
        // exposes only `/constellation/*`.
        app = app.merge(api_routes(server_for_ws.clone(), h.clone()));
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
