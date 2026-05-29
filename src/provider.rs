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
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::cache::TtlCache;
use crate::config::Config;
use crate::hive::{hash_key, Hive};
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
    hive: Option<Arc<Hive>>,
    /// Max providers queried concurrently in aggregate mode (0 = unlimited). Bounds
    /// the burst of outbound requests so a wide `docs` fan-out doesn't trip engine
    /// rate limits.
    max_concurrency: usize,
    /// Per-provider deadline (seconds): a provider that doesn't answer in time is
    /// dropped so one unresponsive/blocked source can't stall the whole search.
    /// 0 = no deadline.
    provider_timeout: u64,
    /// Per-engine quality weights for the composite ranker (default 1.0).
    weights: HashMap<String, f64>,
    /// Extra trusted domains given an authority boost (composite ranker).
    trusted: Vec<String>,
}

impl Registry {
    /// Build the registry from configuration. Unknown provider ids are skipped
    /// with a warning so a typo never takes the whole server down. The result
    /// cache and (optional) hivemind are built by the caller and shared in, since
    /// the hive reads/writes the same cache.
    pub fn from_config(
        cfg: &Config,
        cache: Option<Arc<TtlCache>>,
        hive: Option<Arc<Hive>>,
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
            hive,
            max_concurrency: cfg.search.max_concurrency,
            provider_timeout: cfg.search.provider_timeout_secs,
            weights: cfg.search.engine_weights.clone(),
            trusted: cfg.search.trusted_domains.clone(),
        }
    }

    /// Number of live entries in the shared search cache, if caching is on.
    pub fn cache_len(&self) -> Option<usize> {
        self.cache.as_ref().map(|c| c.keys().len())
    }

    /// The hivemind handle, if the network is enabled — lets skills consult peers
    /// for shared file blobs (e.g. cached PDFs).
    pub(crate) fn hive(&self) -> Option<Arc<Hive>> {
        self.hive.clone()
    }

    /// Human-readable hivemind graph, or a disabled notice. Surfaced by the
    /// `hive_status` tool.
    pub fn hive_report(&self) -> String {
        match &self.hive {
            Some(h) => h.graph_report(),
            None => "Hivemind is disabled ([network].enabled = false).".to_string(),
        }
    }

    /// Per-node hop distances over the mesh, or a disabled notice. Surfaced by the
    /// `hive_peers` tool.
    pub fn hive_peers_report(&self) -> String {
        match &self.hive {
            Some(h) => h.peers_report(),
            None => "Hivemind is disabled ([network].enabled = false).".to_string(),
        }
    }

    /// Per-blob seed ratios (served vs. fetched), or a disabled notice. Surfaced by
    /// the `hive_seeds` tool.
    pub fn hive_seeds_report(&self) -> String {
        match &self.hive {
            Some(h) => h.seed_report(),
            None => "Hivemind is disabled ([network].enabled = false).".to_string(),
        }
    }

    /// Seed ratio (served/fetched bytes) for one blob key-hash, if the hive tracks
    /// it. Used to annotate file-store listings.
    pub(crate) fn blob_seed_ratio(&self, key_hash: &str) -> Option<(u64, u64, Option<f64>)> {
        let s = self.hive.as_ref()?.seed_for(key_hash)?;
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
        let results = search_budgeted(provider.as_ref(), http, query, self.provider_timeout).await;
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
        // Hash the logical key so cache entries and peer lookups share a stable,
        // privacy-preserving id (raw query text never crosses the wire).
        let key = hash_key(&format!(
            "search|{}|{}|{}|{}",
            kind.as_str(),
            plan.strategy.as_str(),
            plan.ranking.as_str(),
            query_key(query),
        ));
        if let Some(hits) = self.cache_get(&key) {
            return (hits, "cache".to_string());
        }
        // Consult-then-fetch: if peers corroborate a result (consensus + capped
        // single-peer influence), trust it and skip re-scraping; otherwise run a
        // normal local search and learn from any peer hits.
        if let Some(hive) = &self.hive {
            let peer_hits = hive.consult(&key).await;
            let trusted = hive.consensus(&peer_hits, query.limit);
            if !trusted.is_empty() {
                hive.update_reputations(&peer_hits, &trusted);
                self.cache_put(&key, &trusted);
                return (trusted, "hive".to_string());
            }
            let (results, label) = self.run_strategy(kind, http, query).await;
            self.cache_put(&key, &results);
            if !peer_hits.is_empty() {
                hive.update_reputations(&peer_hits, &results);
            }
            return (results, label);
        }
        let (results, label) = self.run_strategy(kind, http, query).await;
        self.cache_put(&key, &results);
        (results, label)
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
            let results =
                search_budgeted(provider.as_ref(), http, query, self.provider_timeout).await;
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
        let handles: Vec<_> = self
            .chain(kind)
            .iter()
            .map(|provider| {
                let provider = Arc::clone(provider);
                let http = http.clone();
                let query = query.clone();
                let sem = sem.clone();
                tokio::spawn(async move {
                    // Hold a permit (if a cap is set) for the provider's whole call.
                    let _permit = match sem {
                        Some(s) => s.acquire_owned().await.ok(),
                        None => None,
                    };
                    let id = provider.id();
                    let results = search_budgeted(provider.as_ref(), &http, &query, budget).await;
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

/// Run one provider with an optional per-provider deadline. A provider that
/// times out or errors yields an empty result set (logged) rather than stalling
/// or aborting the whole search — so one unresponsive/blocked source is simply
/// dropped. `budget_secs == 0` disables the deadline.
async fn search_budgeted(
    provider: &dyn SearchProvider,
    http: &Client,
    query: &SearchQuery,
    budget_secs: u64,
) -> Vec<SearchResult> {
    let id = provider.id();
    let run = provider.search(http, query);
    let outcome = if budget_secs == 0 {
        run.await
    } else {
        match tokio::time::timeout(std::time::Duration::from_secs(budget_secs), run).await {
            Ok(res) => res,
            Err(_) => {
                tracing::warn!(
                    provider = id,
                    secs = budget_secs,
                    "provider timed out; dropped"
                );
                return Vec::new();
            }
        }
    };
    match outcome {
        Ok(results) => results,
        Err(e) => {
            tracing::warn!(provider = id, error = %e, "provider failed");
            Vec::new()
        }
    }
}

/// Stable cache-key fragment covering everything about a query that can change
/// its results (text, limit, language/site selectors, and the render flag).
fn query_key(q: &SearchQuery) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        q.render,
        q.limit,
        q.language.as_deref().unwrap_or(""),
        q.site.as_deref().unwrap_or(""),
        q.text,
    )
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
}
