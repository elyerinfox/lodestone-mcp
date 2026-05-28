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

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::Serialize;

use crate::config::Config;
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
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Web => "web",
            ProviderKind::Code => "code",
            ProviderKind::Qa => "qa",
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
    /// Set per call by the model; ignored by providers that don't scrape HTML
    /// and in builds without the `browser` feature.
    #[cfg_attr(not(feature = "browser"), allow(dead_code))]
    pub render: bool,
}

/// A normalized result returned by any provider. Optional fields are populated
/// only when meaningful for the provider's kind.
#[derive(Debug, Default, Clone, Serialize)]
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

/// Holds the configured, ordered providers for each kind and combines them
/// according to the configured [`Strategy`].
pub struct Registry {
    web: Vec<Box<dyn SearchProvider>>,
    code: Vec<Box<dyn SearchProvider>>,
    qa: Vec<Box<dyn SearchProvider>>,
    strategy: Strategy,
}

impl Registry {
    /// Build the registry from configuration. Unknown provider ids are skipped
    /// with a warning so a typo never takes the whole server down.
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            web: build(ProviderKind::Web, &cfg.providers.web, cfg),
            code: build(ProviderKind::Code, &cfg.providers.code, cfg),
            qa: build(ProviderKind::Qa, &cfg.providers.qa, cfg),
            strategy: Strategy::parse(&cfg.search.strategy),
        }
    }

    fn chain(&self, kind: ProviderKind) -> &[Box<dyn SearchProvider>] {
        match kind {
            ProviderKind::Web => &self.web,
            ProviderKind::Code => &self.code,
            ProviderKind::Qa => &self.qa,
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
        match self.strategy {
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
            match provider.search(http, query).await {
                Ok(results) if !results.is_empty() => return (results, provider.id().to_string()),
                Ok(_) => tracing::debug!(
                    provider = provider.id(),
                    kind = provider.kind().as_str(),
                    "no results"
                ),
                Err(e) => tracing::warn!(provider = provider.id(), error = %e, "provider failed"),
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
        let futures = self.chain(kind).iter().map(|provider| {
            let provider = provider.as_ref();
            async move {
                match provider.search(http, query).await {
                    Ok(results) => (provider.id(), results),
                    Err(e) => {
                        tracing::warn!(provider = provider.id(), error = %e, "provider failed");
                        (provider.id(), Vec::new())
                    }
                }
            }
        });
        let per_engine: Vec<(&'static str, Vec<SearchResult>)> =
            futures::future::join_all(futures).await;

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
        (merge(per_engine, query.limit), label)
    }

    /// Human-readable summary of the active providers and strategy.
    pub fn describe(&self) -> String {
        let line = |kind: ProviderKind| {
            let ids: Vec<&str> = self.chain(kind).iter().map(|p| p.id()).collect();
            let value = if ids.is_empty() {
                "(none configured)".to_string()
            } else {
                let sep = match self.strategy {
                    Strategy::Fallback => " → ",
                    Strategy::Aggregate => " + ",
                };
                ids.join(sep)
            };
            format!("{:>4}: {value}", kind.as_str())
        };
        format!(
            "Active providers (strategy: {}):\n{}\n{}\n{}",
            self.strategy.as_str(),
            line(ProviderKind::Web),
            line(ProviderKind::Code),
            line(ProviderKind::Qa),
        )
    }
}

fn build(kind: ProviderKind, ids: &[String], cfg: &Config) -> Vec<Box<dyn SearchProvider>> {
    ids.iter()
        .filter_map(|id| match providers::make(kind, id, cfg) {
            Some(p) => Some(p),
            None => {
                tracing::warn!(kind = kind.as_str(), id, "unknown provider id; skipping");
                None
            }
        })
        .collect()
}

/// Merge per-engine result lists into a single ranked list (SearXNG-style):
/// dedupe by normalized URL, score each unique result by the sum of `1/(rank+1)`
/// across the engines that returned it, and annotate which engines found it.
fn merge(per_engine: Vec<(&'static str, Vec<SearchResult>)>, limit: usize) -> Vec<SearchResult> {
    struct Agg {
        result: SearchResult,
        score: f64,
        engines: Vec<&'static str>,
    }

    let mut map: HashMap<String, Agg> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for (engine, results) in per_engine {
        for (rank, result) in results.into_iter().enumerate() {
            let key = normalize_url(&result.url);
            if key.is_empty() {
                continue;
            }
            let score = 1.0 / (rank as f64 + 1.0);
            match map.get_mut(&key) {
                Some(agg) => {
                    agg.score += score;
                    if !agg.engines.contains(&engine) {
                        agg.engines.push(engine);
                    }
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
                        key,
                        Agg {
                            result,
                            score,
                            engines: vec![engine],
                        },
                    );
                }
            }
        }
    }

    let mut aggs: Vec<Agg> = order.into_iter().filter_map(|k| map.remove(&k)).collect();
    aggs.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    aggs.into_iter()
        .take(limit)
        .map(|mut a| {
            a.result.meta = Some(format!("found by: {}", a.engines.join(", ")));
            a.result
        })
        .collect()
}

fn normalize_url(u: &str) -> String {
    let u = u.split('#').next().unwrap_or(u).trim();
    u.trim_end_matches('/').to_string()
}
