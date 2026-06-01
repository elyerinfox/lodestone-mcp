//! The common provider interface.
//!
//! Every data source — a search engine, a code index, a Q&A site — implements
//! the single [`SearchProvider`] trait. Providers are grouped by [`ProviderKind`]
//! and combined by the [`Registry`] using one of two strategies:
//!
//! * **Fallback** — try providers in order, first non-empty result set wins.
//! * **Aggregate** — query all providers of a kind concurrently, then dedupe and
//!   re-rank the merged results (a built-in SearXNG-style meta-search).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::cache::TtlCache;
use crate::config::Config;
use crate::constellation::{hash_key, Constellation, PeerHit};
use crate::providers;

/// What category of search a provider serves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProviderKind {
    /// General web search.
    Web,
    /// Source-code search (results point at files in repositories).
    Code,
    /// Question/answer sites (e.g. StackExchange network).
    Qa,
    /// Documentation & package registries (crates.io, npm, MDN, …).
    Docs,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Web => "web",
            ProviderKind::Code => "code",
            ProviderKind::Qa => "qa",
            ProviderKind::Docs => "docs",
        }
    }
}

/// How the registry combines the providers of a kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Strategy {
    /// First provider to return results wins.
    Fallback,
    /// Query every provider concurrently and merge/re-rank the results.
    Aggregate,
}

impl Strategy {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "aggregate" | "meta" | "merge" => Strategy::Aggregate,
            _ => Strategy::Fallback,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Strategy::Fallback => "fallback",
            Strategy::Aggregate => "aggregate",
        }
    }
}

/// How aggregated results are re-ranked after dedup. Only used by the
/// `Aggregate` strategy (fallback preserves the winning provider's own order).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ranking {
    /// Multi-signal default: Reciprocal Rank Fusion (weighted, k=60) × consensus
    /// across engines × lexical query relevance × authority, then domain-diversified
    /// (MMR). Stronger and more robust than a plain weighted-position sum.
    Composite,
    /// Sum of reciprocal ranks: Σ 1/(rank+1). Rewards high placement and
    /// cross-engine agreement.
    Reciprocal,
    /// Borda count: Σ (N − rank). Linear positional scoring.
    Borda,
    /// Consensus: rank by how many engines returned a result, best position as
    /// the tiebreak. Favors corroborated results (resists single-engine noise).
    Breadth,
    /// Round-robin: take each engine's 1st, then 2nd, … Maximizes source
    /// diversity rather than scoring.
    Interleave,
}

impl Ranking {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "reciprocal" | "rrf_simple" => Ranking::Reciprocal,
            "borda" => Ranking::Borda,
            "breadth" | "consensus" => Ranking::Breadth,
            "interleave" | "round_robin" | "roundrobin" => Ranking::Interleave,
            // "composite" / "rrf" / "fusion" / anything unknown → the strong default.
            _ => Ranking::Composite,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Ranking::Composite => "composite",
            Ranking::Reciprocal => "reciprocal",
            Ranking::Borda => "borda",
            Ranking::Breadth => "breadth",
            Ranking::Interleave => "interleave",
        }
    }
}

/// A normalized query handed to any provider.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// The user's query text.
    pub text: String,
    /// Optional language hint (code search).
    pub language: Option<String>,
    /// Optional site selector (Q&A; e.g. "stackoverflow").
    pub site: Option<String>,
    /// Maximum number of results to return.
    pub limit: usize,
    /// When true, HTML-scraping providers fetch through the headless browser
    /// (executes JS, bypasses some bot-walls) instead of a plain HTTP request.
    /// Set per call by the model; ignored by providers that don't scrape HTML.
    pub render: bool,
}

/// A normalized result returned by any provider. Optional fields are populated
/// only when meaningful for the provider's kind.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    /// `owner/repo` for code results.
    pub repo: Option<String>,
    /// File path within the repo for code results.
    pub path: Option<String>,
    /// Score/votes for Q&A results.
    pub score: Option<i64>,
    /// Extra one-line metadata (tags, answer counts, engine attribution, …).
    pub meta: Option<String>,
}

/// The common interface implemented by every source. Object-safe via
/// `async_trait`, so providers are stored as `Box<dyn SearchProvider>`.
#[async_trait]
pub trait SearchProvider: Send + Sync {
    /// Stable identifier used in config and result attribution (e.g. "mojeek").
    fn id(&self) -> &'static str;

    /// Which category this provider serves.
    fn kind(&self) -> ProviderKind;

    /// Run the search. Return an empty vec to signal "no results"; return `Err`
    /// for transport/parse failures (logged and skipped).
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>>;
}

/// The resolved strategy + ranking for one kind (after applying any per-kind
/// override on top of the global `[search]` settings).
#[derive(Clone, Copy)]
struct KindPlan {
    strategy: Strategy,
    ranking: Ranking,
}

/// Holds the configured, ordered providers for each kind and combines them
/// according to the resolved per-kind [`KindPlan`].
pub struct Registry {
    web: Vec<Arc<dyn SearchProvider>>,
    code: Vec<Arc<dyn SearchProvider>>,
    qa: Vec<Arc<dyn SearchProvider>>,
    docs: Vec<Arc<dyn SearchProvider>>,
    plan_web: KindPlan,
    plan_code: KindPlan,
    plan_qa: KindPlan,
    plan_docs: KindPlan,
    cache: Option<Arc<TtlCache>>,
    constellation: Option<Arc<Constellation>>,
    /// Max providers queried concurrently in aggregate mode (0 = unlimited). Bounds
    /// the burst of outbound requests so a wide `docs` fan-out doesn't trip engine
    /// rate limits.
    max_concurrency: usize,
    /// Per-provider deadline (seconds): a provider that doesn't answer in time is
    /// dropped so one unresponsive/blocked source can't stall the whole search.
    /// 0 = no deadline.
    provider_timeout: u64,
    /// Per-provider circuit breaker (None when disabled): trips a source that keeps
    /// failing so it's skipped fast instead of re-waiting the deadline every call.
    breakers: Option<Arc<Breakers>>,
    /// Also key searches by a normalized concept signature so reworded-but-equivalent
    /// queries reuse a cached/peer result on an exact-key miss (off by default).
    fuzzy_match: bool,
    /// Optional egress proxy client (built from `[search].proxy`): a second route a
    /// blocked provider is retried through (different egress IP). None = no proxy.
    proxy_http: Option<Client>,
    /// Retry a blocked provider through the headless browser as a last route.
    render_fallback: bool,
    /// Per-engine quality weights for the composite ranker (default 1.0).
    weights: HashMap<String, f64>,
    /// Extra trusted domains given an authority boost (composite ranker).
    trusted: Vec<String>,
}

impl Registry {
    /// Build the registry from configuration. Unknown provider ids are skipped
    /// with a warning so a typo never takes the whole server down. The result
    /// cache and (optional) constellation are built by the caller and shared in, since
    /// the constellation reads/writes the same cache.
    pub fn from_config(
        cfg: &Config,
        cache: Option<Arc<TtlCache>>,
        constellation: Option<Arc<Constellation>>,
    ) -> Self {
        let global_strategy = Strategy::parse(&cfg.search.strategy);
        let global_ranking = Ranking::parse(&cfg.search.ranking);
        // Empty override field → inherit the global value.
        let plan = |k: &crate::config::KindSearch| KindPlan {
            strategy: if k.strategy.trim().is_empty() {
                global_strategy
            } else {
                Strategy::parse(&k.strategy)
            },
            ranking: if k.ranking.trim().is_empty() {
                global_ranking
            } else {
                Ranking::parse(&k.ranking)
            },
        };
        Self {
            web: build(ProviderKind::Web, &cfg.providers.web, cfg),
            code: build(ProviderKind::Code, &cfg.providers.code, cfg),
            qa: build(ProviderKind::Qa, &cfg.providers.qa, cfg),
            docs: build(ProviderKind::Docs, &cfg.providers.docs, cfg),
            plan_web: plan(&cfg.search.web),
            plan_code: plan(&cfg.search.code),
            plan_qa: plan(&cfg.search.qa),
            plan_docs: plan(&cfg.search.docs),
            cache,
            constellation,
            max_concurrency: cfg.search.max_concurrency,
            provider_timeout: cfg.search.provider_timeout_secs,
            breakers: (cfg.search.breaker_threshold > 0).then(|| {
                Arc::new(Breakers::new(
                    cfg.search.breaker_threshold,
                    cfg.search.breaker_cooldown_secs,
                ))
            }),
            fuzzy_match: cfg.search.fuzzy_match,
            proxy_http: build_proxy_client(&cfg.search.proxy, cfg.search.timeout_secs),
            render_fallback: cfg.search.render_fallback,
            weights: cfg.search.engine_weights.clone(),
            trusted: cfg.search.trusted_domains.clone(),
        }
    }

    /// Number of live entries in the shared search cache, if caching is on.
    pub fn cache_len(&self) -> Option<usize> {
        self.cache.as_ref().map(|c| c.keys().len())
    }

    /// The constellation handle, if the network is enabled — lets skills consult peers
    /// for shared file blobs (e.g. cached PDFs).
    pub(crate) fn constellation(&self) -> Option<Arc<Constellation>> {
        self.constellation.clone()
    }

    /// Human-readable constellation graph, or a disabled notice. Surfaced by the
    /// `constellation_status` tool.
    pub fn constellation_report(&self) -> String {
        match &self.constellation {
            Some(h) => h.graph_report(),
            None => "Constellation is disabled ([network].enabled = false).".to_string(),
        }
    }

    /// Per-node hop distances over the mesh, or a disabled notice. Surfaced by the
    /// `constellation_peers` tool.
    pub fn constellation_peers_report(&self) -> String {
        match &self.constellation {
            Some(h) => h.peers_report(),
            None => "Constellation is disabled ([network].enabled = false).".to_string(),
        }
    }

    /// Per-blob seed ratios (served vs. fetched), or a disabled notice. Surfaced by
    /// the `constellation_seeds` tool.
    pub fn constellation_seeds_report(&self) -> String {
        match &self.constellation {
            Some(h) => h.seed_report(),
            None => "Constellation is disabled ([network].enabled = false).".to_string(),
        }
    }

    /// Per-feature capability advertisement for this node and every
    /// peer we've seen a digest from. Surfaced by the new
    /// `constellation_capabilities` tool. `cap_filter` (if set) hides
    /// rows where the named capability is OFF — useful for "who can
    /// do browser work?" lookups.
    pub fn constellation_capabilities_report(&self, cap_filter: Option<&str>) -> String {
        match &self.constellation {
            Some(h) => h.capabilities_report(cap_filter),
            None => "Constellation is disabled ([network].enabled = false).".to_string(),
        }
    }

    /// Seed ratio (served/fetched bytes) for one blob key-hash, if the constellation tracks
    /// it. Used to annotate file-store listings.
    pub(crate) fn blob_seed_ratio(&self, key_hash: &str) -> Option<(u64, u64, Option<f64>)> {
        let s = self.constellation.as_ref()?.seed_for(key_hash)?;
        Some((s.served, s.fetched, s.ratio()))
    }

    fn chain(&self, kind: ProviderKind) -> &[Arc<dyn SearchProvider>] {
        match kind {
            ProviderKind::Web => &self.web,
            ProviderKind::Code => &self.code,
            ProviderKind::Qa => &self.qa,
            ProviderKind::Docs => &self.docs,
        }
    }

    fn plan(&self, kind: ProviderKind) -> KindPlan {
        match kind {
            ProviderKind::Web => self.plan_web,
            ProviderKind::Code => self.plan_code,
            ProviderKind::Qa => self.plan_qa,
            ProviderKind::Docs => self.plan_docs,
        }
    }

    /// Every configured provider as `(kind, id)` — used to expose one direct
    /// tool per provider.
    pub fn list(&self) -> Vec<(ProviderKind, &'static str)> {
        [
            ProviderKind::Web,
            ProviderKind::Code,
            ProviderKind::Qa,
            ProviderKind::Docs,
        ]
        .into_iter()
        .flat_map(|kind| self.chain(kind).iter().map(move |p| (kind, p.id())))
        .collect()
    }

    /// Run a single named provider directly (no chain/strategy). Returns its
    /// results, or an empty vec if the provider isn't configured or errors.
    pub async fn run_one(
        &self,
        kind: ProviderKind,
        id: &str,
        http: &Client,
        query: &SearchQuery,
    ) -> Vec<SearchResult> {
        let Some(provider) = self.chain(kind).iter().find(|p| p.id() == id) else {
            return Vec::new();
        };
        let key = hash_key(&format!(
            "one|{}|{}|{}",
            kind.as_str(),
            id,
            query_key(query)
        ));
        if let Some(hits) = self.cache_get(&key) {
            return hits;
        }
        let results = search_budgeted(
            provider.as_ref(),
            http,
            self.proxy_http.as_ref(),
            self.render_fallback,
            query,
            self.provider_timeout,
            self.breakers.as_deref(),
        )
        .await;
        self.cache_put(&key, &results);
        results
    }

    /// Look up a cached, still-live result list for `key`.
    fn cache_get(&self, key: &str) -> Option<Vec<SearchResult>> {
        let json = self.cache.as_ref()?.get(key)?;
        serde_json::from_str(&json).ok()
    }

    /// Cache a non-empty result list (empty results are never cached, so a
    /// transiently blocked source is retried next time rather than pinned empty).
    fn cache_put(&self, key: &str, results: &[SearchResult]) {
        if results.is_empty() {
            return;
        }
        if let Some(cache) = self.cache.as_ref() {
            if let Ok(json) = serde_json::to_string(results) {
                cache.put(key.to_string(), json);
            }
        }
    }

    /// Run the configured strategy for `kind`, returning the results and a
    /// human-readable description of which engine(s) produced them.
    pub async fn search(
        &self,
        kind: ProviderKind,
        http: &Client,
        query: &SearchQuery,
    ) -> (Vec<SearchResult>, String) {
        let plan = self.plan(kind);
        let envelope = format!(
            "{}|{}|{}",
            kind.as_str(),
            plan.strategy.as_str(),
            plan.ranking.as_str(),
        );
        // Hash the logical key so cache entries and peer lookups share a stable,
        // privacy-preserving id (raw query text never crosses the wire). The text is
        // canonicalized (case/punctuation/stop-words/whitespace folded, order kept)
        // so trivially-reworded queries land on the same key.
        let exact_key = hash_key(&format!("search|{}|{}", envelope, query_key(query)));
        // Optional fuzzy/concept key: an order-independent, stemmed token set so a
        // differently-worded but equivalent query reuses a prior result on a miss.
        let concept_key = if self.fuzzy_match {
            concept_query_key(query).map(|sig| hash_key(&format!("concept|{}|{}", envelope, sig)))
        } else {
            None
        };

        // Try the exact key first (cache → constellation consensus), then the concept key.
        let mut peers: Vec<PeerHit> = Vec::new();
        if let Some(found) = self
            .lookup_key(
                &exact_key,
                query.limit,
                "cache",
                "constellation",
                &mut peers,
            )
            .await
        {
            return found;
        }
        if let Some(ck) = &concept_key {
            if let Some(found) = self
                .lookup_key(
                    ck,
                    query.limit,
                    "cache (fuzzy)",
                    "constellation (fuzzy)",
                    &mut peers,
                )
                .await
            {
                return found;
            }
        }

        // Miss everywhere → run locally, cache under both keys (so this node can
        // serve exact and concept matches), and learn from any peer hits.
        let (results, label) = self.run_strategy(kind, http, query).await;
        self.cache_put(&exact_key, &results);
        if let Some(ck) = &concept_key {
            self.cache_put(ck, &results);
        }
        if let Some(constellation) = &self.constellation {
            if !peers.is_empty() {
                constellation.update_reputations(&peers, &results);
            }
        }
        (results, label)
    }

    /// Look one key up: local cache first, then (if the constellation is on) a peer consult
    /// gated by consensus. Returns the hits + a source label on a hit, or `None`.
    /// Peer responses that didn't reach consensus are appended to `peer_acc` so the
    /// caller can still reward/penalize peers against the eventual local result.
    async fn lookup_key(
        &self,
        key: &str,
        limit: usize,
        label_cache: &str,
        label_hive: &str,
        peer_acc: &mut Vec<PeerHit>,
    ) -> Option<(Vec<SearchResult>, String)> {
        if let Some(hits) = self.cache_get(key) {
            return Some((hits, label_cache.to_string()));
        }
        if let Some(constellation) = &self.constellation {
            let peer_hits = constellation.consult(key).await;
            let trusted = constellation.consensus(&peer_hits, limit);
            if !trusted.is_empty() {
                constellation.update_reputations(&peer_hits, &trusted);
                self.cache_put(key, &trusted);
                return Some((trusted, label_hive.to_string()));
            }
            peer_acc.extend(peer_hits);
        }
        None
    }

    async fn run_strategy(
        &self,
        kind: ProviderKind,
        http: &Client,
        query: &SearchQuery,
    ) -> (Vec<SearchResult>, String) {
        match self.plan(kind).strategy {
            Strategy::Fallback => self.search_fallback(kind, http, query).await,
            Strategy::Aggregate => self.search_aggregate(kind, http, query).await,
        }
    }

    async fn search_fallback(
        &self,
        kind: ProviderKind,
        http: &Client,
        query: &SearchQuery,
    ) -> (Vec<SearchResult>, String) {
        for provider in self.chain(kind) {
            let results = search_budgeted(
                provider.as_ref(),
                http,
                self.proxy_http.as_ref(),
                self.render_fallback,
                query,
                self.provider_timeout,
                self.breakers.as_deref(),
            )
            .await;
            if results.is_empty() {
                tracing::debug!(
                    provider = provider.id(),
                    kind = provider.kind().as_str(),
                    "no results"
                );
            } else {
                return (results, provider.id().to_string());
            }
        }
        (Vec::new(), "none".to_string())
    }

    async fn search_aggregate(
        &self,
        kind: ProviderKind,
        http: &Client,
        query: &SearchQuery,
    ) -> (Vec<SearchResult>, String) {
        // Source every provider in parallel: each runs on its own task so both
        // network I/O and (CPU-bound) HTML parsing overlap across the runtime's
        // worker threads, rather than being polled on a single task. A semaphore
        // caps how many run *at once* so a wide fan-out (e.g. ~20 doc sites, each
        // hitting DuckDuckGo) doesn't burst past engine rate limits; tasks beyond
        // the cap queue for a permit. `max_concurrency == 0` means unlimited.
        let sem = (self.max_concurrency > 0)
            .then(|| Arc::new(tokio::sync::Semaphore::new(self.max_concurrency)));
        let budget = self.provider_timeout;
        let render_fallback = self.render_fallback;
        let handles: Vec<_> = self
            .chain(kind)
            .iter()
            .map(|provider| {
                let provider = Arc::clone(provider);
                let http = http.clone();
                let proxy = self.proxy_http.clone();
                let query = query.clone();
                let sem = sem.clone();
                let breakers = self.breakers.clone();
                tokio::spawn(async move {
                    // Hold a permit (if a cap is set) for the provider's whole call.
                    let _permit = match sem {
                        Some(s) => s.acquire_owned().await.ok(),
                        None => None,
                    };
                    let id = provider.id();
                    let results = search_budgeted(
                        provider.as_ref(),
                        &http,
                        proxy.as_ref(),
                        render_fallback,
                        &query,
                        budget,
                        breakers.as_deref(),
                    )
                    .await;
                    (id, results)
                })
            })
            .collect();

        let mut per_engine: Vec<(&'static str, Vec<SearchResult>)> =
            Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(pair) => per_engine.push(pair),
                Err(e) => tracing::warn!(error = %e, "provider task failed to join"),
            }
        }

        let engines: Vec<&str> = per_engine
            .iter()
            .filter(|(_, r)| !r.is_empty())
            .map(|(id, _)| *id)
            .collect();
        let label = if engines.is_empty() {
            "none".to_string()
        } else {
            format!("aggregate ({})", engines.join("+"))
        };
        let ctx = RankCtx {
            query,
            weights: &self.weights,
            trusted: &self.trusted,
        };
        (
            merge(per_engine, query.limit, self.plan(kind).ranking, &ctx),
            label,
        )
    }

    /// Human-readable summary of the active providers and each kind's strategy.
    pub fn describe(&self) -> String {
        let line = |kind: ProviderKind| {
            let plan = self.plan(kind);
            let ids: Vec<&str> = self.chain(kind).iter().map(|p| p.id()).collect();
            let value = if ids.is_empty() {
                "(none configured)".to_string()
            } else {
                let sep = match plan.strategy {
                    Strategy::Fallback => " → ",
                    Strategy::Aggregate => " + ",
                };
                ids.join(sep)
            };
            let how = if plan.strategy == Strategy::Aggregate {
                format!(
                    "{}, ranking: {}",
                    plan.strategy.as_str(),
                    plan.ranking.as_str()
                )
            } else {
                plan.strategy.as_str().to_string()
            };
            format!("{:>4}: {value}  [{how}]", kind.as_str())
        };
        format!(
            "Active providers:\n{}\n{}\n{}\n{}",
            line(ProviderKind::Web),
            line(ProviderKind::Code),
            line(ProviderKind::Qa),
            line(ProviderKind::Docs),
        )
    }
}

/// Run one provider with an optional per-provider deadline and circuit breaker.
/// A provider that times out or errors yields an empty result set (logged) rather
/// than stalling or aborting the whole search — so one unresponsive/blocked source
/// is simply dropped. When `breakers` is set, a tripped provider is skipped without
/// a network call, and timeouts/errors feed the breaker so a persistently failing
/// source fails fast. `budget_secs == 0` disables the deadline.
async fn search_budgeted(
    provider: &dyn SearchProvider,
    http: &Client,
    proxy: Option<&Client>,
    render_fallback: bool,
    query: &SearchQuery,
    budget_secs: u64,
    breakers: Option<&Breakers>,
) -> Vec<SearchResult> {
    let id = provider.id();
    if let Some(b) = breakers {
        if b.is_open(id) {
            tracing::debug!(provider = id, "circuit open; skipping provider");
            return Vec::new();
        }
    }
    // Try independent egress routes and take the first that yields results — so a
    // source that blocks/rate-limits one route (a tarpitted IP, a bot-wall) can still
    // be reached another way. Each route gets the full deadline; we only escalate
    // when the prior route returns nothing or fails.
    let mut reachable = false;
    let success = |b: Option<&Breakers>| {
        if let Some(b) = b {
            b.record_success(id);
        }
    };

    // 1) Direct (honors the caller's `render` flag as-is).
    match run_route(provider, http, query, budget_secs, "direct").await {
        Reach::Hits(r) => {
            success(breakers);
            return r;
        }
        Reach::Empty => reachable = true,
        Reach::Fail => {}
    }
    // 2) Proxy (different egress IP), same query.
    if let Some(p) = proxy {
        match run_route(provider, p, query, budget_secs, "proxy").await {
            Reach::Hits(r) => {
                success(breakers);
                return r;
            }
            Reach::Empty => reachable = true,
            Reach::Fail => {}
        }
    }
    // 3) Headless browser (a real browser bypasses many bot-walls), unless already
    //    rendering.
    if render_fallback && !query.render {
        let q = SearchQuery {
            render: true,
            ..query.clone()
        };
        match run_route(provider, http, &q, budget_secs, "render").await {
            Reach::Hits(r) => {
                success(breakers);
                return r;
            }
            Reach::Empty => reachable = true,
            Reach::Fail => {}
        }
    }

    if let Some(b) = breakers {
        // Reachable on any route (even empty) clears the streak; all-routes-failed trips it.
        if reachable {
            b.record_success(id);
        } else {
            b.record_failure(id);
        }
    }
    Vec::new()
}

/// Outcome of one egress route attempt.
enum Reach {
    /// Non-empty results.
    Hits(Vec<SearchResult>),
    /// Reachable but no results (a valid empty response).
    Empty,
    /// Timed out or errored.
    Fail,
}

/// Run one route (a given client + query) within the deadline.
async fn run_route(
    provider: &dyn SearchProvider,
    client: &Client,
    query: &SearchQuery,
    budget_secs: u64,
    route: &str,
) -> Reach {
    let id = provider.id();
    let run = provider.search(client, query);
    let outcome = if budget_secs == 0 {
        run.await
    } else {
        match tokio::time::timeout(Duration::from_secs(budget_secs), run).await {
            Ok(res) => res,
            Err(_) => {
                tracing::warn!(provider = id, route, secs = budget_secs, "route timed out");
                return Reach::Fail;
            }
        }
    };
    match outcome {
        Ok(r) if !r.is_empty() => Reach::Hits(r),
        Ok(_) => Reach::Empty,
        Err(e) => {
            tracing::warn!(provider = id, route, error = %e, "route failed");
            Reach::Fail
        }
    }
}

/// Build an egress proxy client (http/https/socks5/socks5h) sharing the main UA +
/// timeout. None when unset; logs and returns None on a bad URL so a typo can't take
/// the proxy route (or the server) down.
fn build_proxy_client(proxy: &str, timeout_secs: u64) -> Option<Client> {
    let url = proxy.trim();
    if url.is_empty() {
        return None;
    }
    let built = reqwest::Proxy::all(url).and_then(|p| {
        Client::builder()
            .user_agent(crate::USER_AGENT)
            .timeout(Duration::from_secs(timeout_secs.max(1)))
            .proxy(p)
            .build()
    });
    match built {
        Ok(c) => {
            tracing::info!(proxy = url, "search egress proxy route enabled");
            Some(c)
        }
        Err(e) => {
            tracing::warn!(proxy = url, error = %e, "invalid [search].proxy; proxy route disabled");
            None
        }
    }
}

/// Per-provider circuit breaker. Tracks consecutive *reachability* failures
/// (timeouts / transport / parse errors) per provider id; once `threshold` is hit,
/// the provider is skipped until `cooldown` elapses — so a source actively blocking
/// this egress IP fails fast instead of burning the per-provider deadline on every
/// call. Any successful response (including an empty result set) resets the streak.
struct Breakers {
    threshold: u32,
    cooldown: Duration,
    state: Mutex<HashMap<&'static str, BreakerState>>,
}

#[derive(Default)]
struct BreakerState {
    failures: u32,
    open_until: Option<Instant>,
}

impl Breakers {
    fn new(threshold: u32, cooldown_secs: u64) -> Self {
        Self {
            threshold,
            cooldown: Duration::from_secs(cooldown_secs),
            state: Mutex::new(HashMap::new()),
        }
    }

    /// True if `id` is currently tripped and its cooldown hasn't elapsed. Once the
    /// cooldown passes the breaker resets to closed, so the next call probes the
    /// source (half-open): if it fails again the breaker simply re-trips.
    fn is_open(&self, id: &str) -> bool {
        let mut state = self.state.lock().unwrap();
        let Some(s) = state.get_mut(id) else {
            return false;
        };
        match s.open_until {
            Some(until) if Instant::now() < until => true,
            Some(_) => {
                s.failures = 0;
                s.open_until = None;
                false
            }
            None => false,
        }
    }

    /// Record a reachable response: clears the failure streak and closes the breaker.
    fn record_success(&self, id: &'static str) {
        let mut state = self.state.lock().unwrap();
        if let Some(s) = state.get_mut(id) {
            s.failures = 0;
            s.open_until = None;
        }
    }

    /// Record a reachability failure; trips the breaker once the threshold is hit.
    fn record_failure(&self, id: &'static str) {
        let mut state = self.state.lock().unwrap();
        let s = state.entry(id).or_default();
        s.failures = s.failures.saturating_add(1);
        if s.failures >= self.threshold && s.open_until.is_none() {
            s.open_until = Some(Instant::now() + self.cooldown);
            tracing::warn!(
                provider = id,
                failures = s.failures,
                cooldown_secs = self.cooldown.as_secs(),
                "circuit breaker opened; skipping provider during cooldown"
            );
        }
    }
}

/// Stable cache-key fragment covering everything about a query that can change
/// its results (text, limit, language/site selectors, and the render flag). The
/// text is canonicalized so trivially-reworded queries share a key.
fn query_key(q: &SearchQuery) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        q.render,
        q.limit,
        q.language.as_deref().unwrap_or(""),
        q.site.as_deref().unwrap_or(""),
        canonical_query(&q.text),
    )
}

/// Like [`query_key`] but with the text reduced to an order-independent **concept
/// signature** (stemmed token set), for the optional fuzzy-match key. Returns `None`
/// when the query has no content words to key on (so we never build a useless key).
fn concept_query_key(q: &SearchQuery) -> Option<String> {
    let toks = concept_tokens(&q.text);
    if toks.is_empty() {
        return None;
    }
    Some(format!(
        "{}|{}|{}|{}|{}",
        q.render,
        q.limit,
        q.language.as_deref().unwrap_or(""),
        q.site.as_deref().unwrap_or(""),
        toks.join(" "),
    ))
}

/// Common, low-signal words dropped from cache keys so phrasing differences don't
/// fragment otherwise-identical queries. Deliberately small and query-oriented.
const STOPWORDS: &[&str] = &[
    "a", "an", "the", "of", "in", "on", "for", "and", "or", "is", "are", "be", "how", "do", "does",
    "did", "i", "my", "me", "with", "what", "whats", "when", "where", "why", "which", "can",
    "could", "should", "would", "please", "get", "got", "using", "use", "via", "about", "any",
    "some", "this", "that", "it",
];

fn is_stopword(t: &str) -> bool {
    STOPWORDS.contains(&t)
}

/// Lowercase a token and strip surrounding punctuation (keeps token-internal chars
/// like `c++` or `node.js` intact — only the ends are trimmed).
fn clean_token(raw: &str) -> String {
    raw.to_ascii_lowercase()
        .trim_matches(|c: char| ".,!?;:\"'`()[]{}".contains(c))
        .to_string()
}

/// The shared, runtime-learned single-token alias map. Read on every
/// `canonical_query`/`concept_tokens` token; written by the `synonym_*` tools
/// in `crate::skills::memory`. Out of the box it is **empty** — no hardcoded
/// table — and the model/user grows it as they learn (`synonym_add`).
///
/// Stored as `OnceLock<Arc<RwLock<HashMap<…>>>>` so it can be installed once at
/// startup (after the on-disk store is loaded) and shared across threads. Reads
/// take a read-lock and clone the matched value (RwLock allows concurrent
/// reads, so frequent canonicalization isn't serialized).
static SYNONYMS: std::sync::OnceLock<std::sync::Arc<std::sync::RwLock<HashMap<String, String>>>> =
    std::sync::OnceLock::new();

/// Install (or recover) the shared synonym store. The first caller wins; later
/// callers get the previously-installed `Arc` back. This way every `Memory`
/// instance ends up holding **the same `Arc`** as the global, so writes through
/// `mem.synonyms.write()` are visible to `canonical_query`/`concept_tokens`.
pub(crate) fn install_synonym_store(
    store: std::sync::Arc<std::sync::RwLock<HashMap<String, String>>>,
) -> std::sync::Arc<std::sync::RwLock<HashMap<String, String>>> {
    SYNONYMS.get_or_init(|| store).clone()
}

/// Look up `t` in the runtime synonym map. Returns `t` unchanged if no map is
/// installed (server started without `[memory]`) or no alias matches. Lowercase
/// keys/values are expected (the `synonym_add` tool enforces this).
fn fold_synonym(t: &str) -> String {
    if let Some(store) = SYNONYMS.get() {
        if let Ok(map) = store.read() {
            if let Some(v) = map.get(t) {
                return v.clone();
            }
        }
    }
    t.to_string()
}

/// Order-preserving canonical form of a query: lowercased, de-punctuated, with
/// stop-words and excess whitespace removed. Word order is **kept**, so
/// direction-sensitive phrasings stay distinct (e.g. "json to yaml" ≠ "yaml to
/// json"). Falls back to a whitespace-normalized lowercasing if every token was a
/// stop-word, so the key is never empty.
pub(crate) fn canonical_query(text: &str) -> String {
    let toks: Vec<String> = text
        .split_whitespace()
        .map(clean_token)
        .map(|t| fold_synonym(&t))
        .filter(|t| !t.is_empty() && !is_stopword(t))
        .collect();
    if toks.is_empty() {
        return text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_lowercase();
    }
    toks.join(" ")
}

/// Very light suffix stemmer (no dependency): folds common inflections so e.g.
/// "parsing"/"parsed"/"parses" collapse toward "parse". Conservative — only strips
/// when a reasonable stem remains.
fn stem(t: &str) -> String {
    for suf in ["ing", "ed", "s"] {
        if t.len() > suf.len() + 2 && t.ends_with(suf) {
            return t[..t.len() - suf.len()].to_string();
        }
    }
    t.to_string()
}

/// The order-independent concept token set of a query: cleaned, stop-worded,
/// stemmed, sorted, de-duplicated. Two differently-ordered or differently-inflected
/// phrasings of the same content words produce the same set.
pub(crate) fn concept_tokens(text: &str) -> Vec<String> {
    let mut toks: Vec<String> = text
        .split_whitespace()
        .map(clean_token)
        .map(|t| fold_synonym(&t))
        .filter(|t| !t.is_empty() && !is_stopword(t))
        .map(|t| stem(&t))
        .collect();
    toks.sort();
    toks.dedup();
    toks
}

fn build(kind: ProviderKind, ids: &[String], cfg: &Config) -> Vec<Arc<dyn SearchProvider>> {
    ids.iter()
        .filter_map(|id| match providers::make(kind, id, cfg) {
            Some(p) => Some(Arc::from(p)),
            None => {
                tracing::warn!(kind = kind.as_str(), id, "unknown provider id; skipping");
                None
            }
        })
        .collect()
}

/// One deduplicated result plus the `(engine, rank)` sources that produced it.
struct Agg {
    result: SearchResult,
    /// Each engine that returned this result and the 0-based position it gave it.
    sources: Vec<(&'static str, usize)>,
}

impl Agg {
    /// Distinct engines (in first-seen order) that returned this result.
    fn engines(&self) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = Vec::new();
        for (e, _) in &self.sources {
            if !out.contains(e) {
                out.push(e);
            }
        }
        out
    }

    /// Return the merged result. Engine provenance is intentionally NOT stamped
    /// onto `meta` — it's redundant with the result-set header and would clobber
    /// providers' real metadata (Q&A answer counts/tags, package versions). The
    /// engines that produced it still drive ranking.
    fn finish(self) -> SearchResult {
        self.result
    }
}

/// Context the composite ranker uses beyond positions: the query (for lexical
/// relevance), per-engine weights, and extra trusted domains.
pub(crate) struct RankCtx<'a> {
    pub query: &'a SearchQuery,
    pub weights: &'a HashMap<String, f64>,
    pub trusted: &'a [String],
}

/// Merge per-engine result lists into a single list: dedupe by normalized URL,
/// then order according to `ranking`. Each result is annotated with the engines
/// that found it.
fn merge(
    per_engine: Vec<(&'static str, Vec<SearchResult>)>,
    limit: usize,
    ranking: Ranking,
    ctx: &RankCtx,
) -> Vec<SearchResult> {
    let mut map: HashMap<String, Agg> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    // Per-engine ordered keys, for the interleave ranking.
    let mut per_engine_keys: Vec<Vec<String>> = Vec::with_capacity(per_engine.len());
    let mut max_len = 0usize;

    for (engine, results) in per_engine {
        let mut keys = Vec::with_capacity(results.len());
        for (rank, result) in results.into_iter().enumerate() {
            let key = normalize_url(&result.url);
            if key.is_empty() {
                continue;
            }
            match map.get_mut(&key) {
                Some(agg) => {
                    agg.sources.push((engine, rank));
                    if result.snippet.len() > agg.result.snippet.len() {
                        agg.result.snippet = result.snippet;
                    }
                    if agg.result.title.is_empty() {
                        agg.result.title = result.title;
                    }
                }
                None => {
                    order.push(key.clone());
                    map.insert(
                        key.clone(),
                        Agg {
                            result,
                            sources: vec![(engine, rank)],
                        },
                    );
                }
            }
            keys.push(key);
        }
        max_len = max_len.max(keys.len());
        per_engine_keys.push(keys);
    }

    match ranking {
        Ranking::Interleave => interleave(map, &per_engine_keys, max_len, limit),
        Ranking::Composite => {
            let aggs: Vec<Agg> = order.into_iter().filter_map(|k| map.remove(&k)).collect();
            composite(aggs, limit, ctx)
        }
        _ => {
            let mut aggs: Vec<Agg> = order.into_iter().filter_map(|k| map.remove(&k)).collect();
            aggs.sort_by(|a, b| {
                score(b, ranking, max_len)
                    .partial_cmp(&score(a, ranking, max_len))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            aggs.into_iter().take(limit).map(Agg::finish).collect()
        }
    }
}

/// Score for the comparison-based rankings (higher = better).
fn score(agg: &Agg, ranking: Ranking, n: usize) -> f64 {
    let reciprocal: f64 = agg
        .sources
        .iter()
        .map(|(_, r)| 1.0 / (*r as f64 + 1.0))
        .sum();
    match ranking {
        Ranking::Borda => agg.sources.iter().map(|(_, r)| (n - r) as f64).sum(),
        // Consensus first (engine count dominates), reciprocal as the tiebreak.
        Ranking::Breadth => agg.engines().len() as f64 * 1_000.0 + reciprocal,
        // Reciprocal / Composite (composite handled elsewhere) / Interleave.
        _ => reciprocal,
    }
}

/// Reciprocal Rank Fusion constant. The canonical k≈60 damps the dominance of
/// rank-0 results, making fusion across engines far more robust than 1/(rank+1).
const RRF_K: f64 = 60.0;
/// Per extra corroborating engine, multiply the score by (1 + this).
const CONSENSUS_BONUS: f64 = 0.25;
/// Weight of lexical query relevance and of authority in the composite product.
const LEXICAL_WEIGHT: f64 = 0.5;
/// Each repeat of a domain in the output multiplies its score by this (MMR-style
/// diversification → broader, less redundant results).
const DIVERSITY_DECAY: f64 = 0.6;

/// High-signal developer domains given a small authority boost by default; users
/// can extend this via `[search].trusted_domains`.
const BUILTIN_TRUSTED: &[&str] = &[
    "stackoverflow.com",
    "developer.mozilla.org",
    "docs.rs",
    "doc.rust-lang.org",
    "rust-lang.org",
    "github.com",
    "docs.python.org",
    "pkg.go.dev",
    "kubernetes.io",
    "wikipedia.org",
    "man7.org",
];

/// The composite ranker: score each result by weighted RRF × consensus × lexical
/// relevance × authority, then greedily select with a per-domain decay so one site
/// can't monopolize the top results.
fn composite(aggs: Vec<Agg>, limit: usize, ctx: &RankCtx) -> Vec<SearchResult> {
    let terms = query_terms(&ctx.query.text);
    // (base score, domain, finished result)
    let mut scored: Vec<(f64, String, SearchResult)> = aggs
        .into_iter()
        .map(|agg| {
            let rrf: f64 = agg
                .sources
                .iter()
                .map(|(e, r)| engine_weight(e, ctx.weights) / (RRF_K + *r as f64))
                .sum();
            let consensus = 1.0 + CONSENSUS_BONUS * (agg.engines().len().saturating_sub(1) as f64);
            let haystack = format!("{} {}", agg.result.title, agg.result.snippet);
            let lexical = 1.0 + LEXICAL_WEIGHT * term_coverage(&terms, &haystack);
            let authority = 1.0 + authority(&agg.result, ctx.trusted);
            let base = rrf * consensus * lexical * authority;
            let domain = domain_of(&agg.result.url);
            (base, domain, agg.finish())
        })
        .collect();

    // Greedy MMR-style selection: each already-picked occurrence of a domain
    // decays that domain's remaining candidates.
    let mut out = Vec::with_capacity(limit.min(scored.len()));
    let mut domain_count: HashMap<String, i32> = HashMap::new();
    while out.len() < limit && !scored.is_empty() {
        let mut best_i = 0;
        let mut best_eff = f64::MIN;
        for (i, (base, domain, _)) in scored.iter().enumerate() {
            let seen = domain_count.get(domain).copied().unwrap_or(0);
            let eff = base * DIVERSITY_DECAY.powi(seen);
            if eff > best_eff {
                best_eff = eff;
                best_i = i;
            }
        }
        let (_, domain, result) = scored.remove(best_i);
        *domain_count.entry(domain).or_insert(0) += 1;
        out.push(result);
    }
    out
}

fn engine_weight(engine: &str, weights: &HashMap<String, f64>) -> f64 {
    weights.get(engine).copied().unwrap_or(1.0).max(0.0)
}

/// Fraction of distinct query terms that appear (as substrings) in `text`. 0 when
/// there are no usable terms (so it contributes a neutral factor).
fn term_coverage(terms: &[String], text: &str) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let hay = text.to_ascii_lowercase();
    let hits = terms.iter().filter(|t| hay.contains(t.as_str())).count();
    hits as f64 / terms.len() as f64
}

/// Lowercased query tokens worth matching: drops search operators (`site:` …),
/// quotes/punctuation, and very short tokens.
fn query_terms(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tok in text.split_whitespace() {
        if tok.contains(':') {
            continue; // operator like site:/lang:
        }
        let t: String = tok
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if t.len() >= 2 && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// Host of a URL, lowercased, without a leading `www.`.
fn domain_of(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(u) => u
            .host_str()
            .map(|h| h.trim_start_matches("www.").to_ascii_lowercase())
            .unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// Small additive authority signal in roughly [0, ~0.6]: HTTPS, a trusted domain,
/// resolved code (repo) hits, and Q&A vote count.
fn authority(result: &SearchResult, trusted: &[String]) -> f64 {
    let mut a = 0.0;
    if result.url.starts_with("https://") {
        a += 0.05;
    }
    let domain = domain_of(&result.url);
    let is_trusted = !domain.is_empty()
        && (BUILTIN_TRUSTED
            .iter()
            .any(|d| domain == *d || domain.ends_with(&format!(".{d}")))
            || trusted.iter().any(|d| {
                let d = d.trim_start_matches("www.").to_ascii_lowercase();
                domain == d || domain.ends_with(&format!(".{d}"))
            }));
    if is_trusted {
        a += 0.15;
    }
    if result.repo.is_some() {
        a += 0.05;
    }
    if let Some(votes) = result.score {
        a += (votes.clamp(0, 100) as f64 / 100.0) * 0.3;
    }
    a
}

/// Round-robin across engines: 1st of each, then 2nd, … skipping duplicates.
fn interleave(
    mut map: HashMap<String, Agg>,
    per_engine_keys: &[Vec<String>],
    max_len: usize,
    limit: usize,
) -> Vec<SearchResult> {
    let mut out = Vec::new();
    for i in 0..max_len {
        for keys in per_engine_keys {
            if let Some(key) = keys.get(i) {
                if let Some(agg) = map.remove(key) {
                    out.push(agg.finish());
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
    }
    out
}

pub(crate) fn normalize_url(u: &str) -> String {
    let u = u.split('#').next().unwrap_or(u).trim();
    u.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(url: &str) -> SearchResult {
        SearchResult {
            url: url.to_string(),
            title: url.to_string(),
            ..Default::default()
        }
    }

    fn hit_t(url: &str, title: &str) -> SearchResult {
        SearchResult {
            url: url.to_string(),
            title: title.to_string(),
            ..Default::default()
        }
    }

    fn engine(id: &'static str, urls: &[&str]) -> (&'static str, Vec<SearchResult>) {
        (id, urls.iter().map(|u| hit(u)).collect())
    }

    fn urls(results: &[SearchResult]) -> Vec<&str> {
        results.iter().map(|r| r.url.as_str()).collect()
    }

    /// Owns the values a `RankCtx` borrows, so tests can build one cheaply.
    struct TestCtx {
        q: SearchQuery,
        w: HashMap<String, f64>,
        t: Vec<String>,
    }
    impl TestCtx {
        fn new(text: &str) -> Self {
            Self {
                q: SearchQuery {
                    text: text.to_string(),
                    language: None,
                    site: None,
                    limit: 10,
                    render: false,
                },
                w: HashMap::new(),
                t: Vec::new(),
            }
        }
        fn ctx(&self) -> RankCtx<'_> {
            RankCtx {
                query: &self.q,
                weights: &self.w,
                trusted: &self.t,
            }
        }
    }

    #[test]
    fn reciprocal_rewards_agreement_and_placement() {
        // y: a@1 (0.5) + b@0 (1.0) = 1.5 ; x: 1.0 ; z: 0.5
        let tc = TestCtx::new("");
        let per = vec![engine("a", &["x", "y"]), engine("b", &["y", "z"])];
        let out = merge(per, 10, Ranking::Reciprocal, &tc.ctx());
        assert_eq!(urls(&out), ["y", "x", "z"]);
    }

    #[test]
    fn breadth_prefers_corroboration_over_placement() {
        // t appears in both engines but only deep (ranks 3 & 3); x is a single
        // engine's #1. Reciprocal ranks x first; breadth ranks the corroborated t.
        let tc = TestCtx::new("");
        let per = vec![
            engine("a", &["x", "p", "q", "t"]),
            engine("b", &["u", "v", "w", "t"]),
        ];
        let reciprocal = merge(per.clone(), 10, Ranking::Reciprocal, &tc.ctx());
        assert_eq!(reciprocal[0].url, "x");

        let breadth = merge(per, 10, Ranking::Breadth, &tc.ctx());
        assert_eq!(breadth[0].url, "t");
    }

    #[test]
    fn interleave_round_robins_across_engines() {
        let tc = TestCtx::new("");
        let per = vec![engine("a", &["a1", "a2", "a3"]), engine("b", &["b1", "b2"])];
        let out = merge(per, 10, Ranking::Interleave, &tc.ctx());
        assert_eq!(urls(&out), ["a1", "b1", "a2", "b2", "a3"]);
    }

    #[test]
    fn dedupes_by_normalized_url() {
        let tc = TestCtx::new("");
        let per = vec![
            engine("a", &["https://x.test/p"]),
            engine("b", &["https://x.test/p/"]), // trailing slash → same
        ];
        let out = merge(per, 10, Ranking::Reciprocal, &tc.ctx());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://x.test/p");
        // Engine provenance is not stamped onto results (no meta clutter).
        assert!(out[0].meta.is_none());
    }

    #[test]
    fn composite_diversifies_by_domain() {
        // One engine returns two results from d1 then one from d2. A naive
        // position sort would keep both d1 hits on top; the composite ranker
        // demotes the repeated domain so d2 surfaces second.
        let tc = TestCtx::new("");
        let per = vec![engine(
            "a",
            &["https://d1.com/1", "https://d1.com/2", "https://d2.com/1"],
        )];
        let out = merge(per, 10, Ranking::Composite, &tc.ctx());
        assert_eq!(
            urls(&out),
            ["https://d1.com/1", "https://d2.com/1", "https://d1.com/2"]
        );
    }

    #[test]
    fn composite_rewards_lexical_relevance() {
        // Same domain, same engine; the deeper result whose title matches the
        // query should outrank the shallow off-topic one.
        let tc = TestCtx::new("rust async");
        let per = vec![(
            "a",
            vec![
                hit_t("https://x.com/a", "unrelated city guide"),
                hit_t("https://x.com/b", "rust async runtime guide"),
            ],
        )];
        let out = merge(per, 10, Ranking::Composite, &tc.ctx());
        assert_eq!(out[0].url, "https://x.com/b");
    }

    #[test]
    fn breaker_trips_after_threshold_and_resets_on_success() {
        // Threshold 3, long cooldown so it stays open until we reset it.
        let b = Breakers::new(3, 600);
        assert!(!b.is_open("ddg"));
        b.record_failure("ddg");
        b.record_failure("ddg");
        // Below threshold: still closed.
        assert!(!b.is_open("ddg"));
        b.record_failure("ddg");
        // Threshold reached: tripped.
        assert!(b.is_open("ddg"));
        // A reachable response closes it again and clears the streak.
        b.record_success("ddg");
        assert!(!b.is_open("ddg"));
        // Other providers are unaffected.
        assert!(!b.is_open("mojeek"));
    }

    #[test]
    fn breaker_cooldown_expiry_reopens_for_a_probe() {
        // Zero cooldown: the moment it trips, the window is already past, so the
        // next is_open() check half-opens (resets) and lets a probe through.
        let b = Breakers::new(1, 0);
        b.record_failure("ddg");
        // Cooldown of 0 means open_until is already <= now → treated as elapsed.
        assert!(!b.is_open("ddg"));
    }

    #[test]
    fn canonical_query_folds_rewording_but_keeps_order() {
        // Case, punctuation, stop-words, and whitespace all fold to one form…
        assert_eq!(
            canonical_query("How do I parse JSON in Rust?"),
            canonical_query("parse json   rust")
        );
        assert_eq!(canonical_query("Parse JSON in Rust"), "parse json rust");
        // …but word order is preserved, so direction-sensitive queries stay distinct.
        assert_ne!(
            canonical_query("convert json to yaml"),
            canonical_query("convert yaml to json")
        );
        // All-stopword input still yields a stable, non-empty key.
        assert_eq!(canonical_query("how do I"), "how do i");
    }

    #[test]
    fn concept_tokens_are_order_and_inflection_independent() {
        // Word order doesn't matter — same content words → same concept set.
        assert_eq!(
            concept_tokens("parsing JSON files"),
            concept_tokens("files parsing json")
        );
        // The light stemmer folds same-root inflections (-ing / -ed / -s).
        assert_eq!(concept_tokens("parsed"), concept_tokens("parsing"));
        assert_eq!(concept_tokens("files"), concept_tokens("file"));
        // Sorted + de-duped.
        assert_eq!(concept_tokens("rust rust json"), vec!["json", "rust"]);
    }

    #[test]
    fn synonym_store_folds_when_installed() {
        // Out of the box the runtime store is empty — fold_synonym returns the
        // token unchanged. The test installs a seed map (the OnceLock keeps the
        // first store, so this is idempotent across the test process) and then
        // checks reworded queries collapse to the same canonical form.
        let map: std::sync::Arc<std::sync::RwLock<HashMap<String, String>>> =
            std::sync::Arc::new(std::sync::RwLock::new(HashMap::from([
                ("k8s".to_string(), "kubernetes".to_string()),
                ("ssl".to_string(), "tls".to_string()),
                ("gh".to_string(), "github".to_string()),
            ])));
        install_synonym_store(map);
        assert_eq!(
            canonical_query("k8s deployment"),
            canonical_query("kubernetes deployment")
        );
        assert_eq!(canonical_query("nginx ssl"), canonical_query("nginx tls"));
        assert_eq!(canonical_query("gh repo"), canonical_query("github repo"));
        assert_eq!(
            concept_tokens("k8s deploy"),
            concept_tokens("kubernetes deploy")
        );
        // Non-aliased tokens pass through untouched.
        assert_eq!(canonical_query("plain rust query"), "plain rust query");
    }

    #[test]
    fn concept_query_key_none_when_no_content_words() {
        let q = SearchQuery {
            text: "how do I".to_string(),
            language: None,
            site: None,
            limit: 5,
            render: false,
        };
        assert!(concept_query_key(&q).is_none());
        let q2 = SearchQuery {
            text: "parse json".to_string(),
            ..q
        };
        assert!(concept_query_key(&q2).is_some());
    }
}
