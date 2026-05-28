//! The hivemind — an opt-in peer-to-peer layer that lets instances consult each
//! other's caches before scraping the open web. The network is a *helper*, never
//! a dependency: with zero peers everything still works locally.
//!
//! Trust model (anti-poisoning): peer data is untrusted. A result is only
//! returned without a local search when **multiple** peers corroborate it
//! (`min_agreement`), each peer's contribution is capped (`max_results_per_peer`),
//! and peers are weighted by a reputation score earned by agreeing with consensus
//! / local truth. Only hashes of query keys cross the wire — never raw queries —
//! and responses carry only cached search results (public web data), never
//! secrets.

mod bloom;
mod mdns;

pub(crate) use bloom::hash_key;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::cache::TtlCache;
use crate::config::Network as NetworkConfig;
use crate::provider::{normalize_url, SearchResult};
use crate::util::ct_eq;

use bloom::BloomFilter;

/// What a node advertises: a Bloom filter of the hashes it currently has cached.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Digest {
    pub node_id: String,
    pub generation: u64,
    pub count: usize,
    pub bloom: BloomFilter,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct QueryReq {
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct QueryResp {
    pub hits: Vec<SearchResult>,
}

/// One peer's response to a consult.
pub(crate) struct PeerHit {
    pub url: String,
    pub reputation: f64,
    pub hits: Vec<SearchResult>,
}

struct Peer {
    url: String,
    bloom: Option<BloomFilter>,
    reputation: f64,
}

impl Peer {
    fn new(url: String) -> Self {
        Self {
            url,
            bloom: None,
            reputation: 0.5,
        }
    }
}

pub(crate) struct Hive {
    cfg: NetworkConfig,
    node_id: String,
    http: Client,
    cache: Arc<TtlCache>,
    peers: Mutex<HashMap<String, Peer>>,
}

impl Hive {
    /// Build the hive from config and the shared result cache. Seeds the static
    /// peer list; mDNS (if enabled) adds more at runtime.
    pub(crate) fn new(cfg: &NetworkConfig, cache: Arc<TtlCache>) -> Arc<Self> {
        let node_id = if cfg.node_id.trim().is_empty() {
            random_id()
        } else {
            cfg.node_id.trim().to_string()
        };
        let http = Client::builder()
            .user_agent("lodestone-hive")
            .timeout(Duration::from_millis(cfg.request_timeout_ms.max(100)))
            .build()
            .unwrap_or_else(|_| Client::new());
        let mut peers = HashMap::new();
        for url in &cfg.peers {
            let u = normalize_base(url);
            if !u.is_empty() {
                peers.insert(u.clone(), Peer::new(u));
            }
        }
        Arc::new(Self {
            cfg: cfg.clone(),
            node_id,
            http,
            cache,
            peers: Mutex::new(peers),
        })
    }

    pub(crate) fn node_id(&self) -> &str {
        &self.node_id
    }

    pub(crate) fn advertise_port(&self, bind_port: u16) -> u16 {
        if self.cfg.advertise_port != 0 {
            self.cfg.advertise_port
        } else {
            bind_port
        }
    }

    /// True if a presented bearer token satisfies the configured network token
    /// (always true when no token is configured). Constant-time comparison.
    pub(crate) fn token_ok(&self, presented: Option<&str>) -> bool {
        if self.cfg.token.is_empty() {
            return true;
        }
        presented.is_some_and(|t| ct_eq(t.as_bytes(), self.cfg.token.as_bytes()))
    }

    /// Build this node's digest from the live cache keys.
    pub(crate) fn digest(&self) -> Digest {
        let keys = self.cache.keys();
        Digest {
            node_id: self.node_id.clone(),
            generation: now_secs(),
            count: keys.len(),
            bloom: BloomFilter::from_keys(&keys),
        }
    }

    /// Answer a peer's query: our cached hits for `key_hash`, if any.
    pub(crate) fn local_lookup(&self, key_hash: &str) -> Vec<SearchResult> {
        self.cache
            .get(key_hash)
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    /// Register a peer discovered at runtime (e.g. via mDNS).
    pub(crate) fn add_peer(&self, url: &str) {
        let u = normalize_base(url);
        if u.is_empty() {
            return;
        }
        let mut peers = self.peers.lock().unwrap();
        peers.entry(u.clone()).or_insert_with(|| Peer::new(u));
    }

    /// Ask peers (whose Bloom filter says they *might* have it) for `key_hash`.
    /// Bounded by `max_peers` and the per-request timeout; each peer's list is
    /// capped. Never relays — a peer is asked only for its own cache.
    pub(crate) async fn consult(&self, key_hash: &str) -> Vec<PeerHit> {
        let candidates: Vec<(String, f64)> = {
            let peers = self.peers.lock().unwrap();
            peers
                .values()
                .filter(|p| p.bloom.as_ref().is_some_and(|b| b.maybe_contains(key_hash)))
                .map(|p| (p.url.clone(), p.reputation))
                .take(self.cfg.max_peers.max(1))
                .collect()
        };
        if candidates.is_empty() {
            return Vec::new();
        }
        let cap = self.cfg.max_results_per_peer.max(1);
        let futs = candidates.into_iter().map(|(url, reputation)| {
            let http = self.http.clone();
            let token = self.cfg.token.clone();
            let key = key_hash.to_string();
            async move {
                match query_peer(&http, &url, &token, &key, cap).await {
                    Ok(hits) if !hits.is_empty() => Some(PeerHit {
                        url,
                        reputation,
                        hits,
                    }),
                    _ => None,
                }
            }
        });
        futures::future::join_all(futs)
            .await
            .into_iter()
            .flatten()
            .collect()
    }

    /// Consensus merge of peer hits into a *trusted* result list. A URL must be
    /// corroborated by at least `min_agreement` distinct peers; results are
    /// ordered by (corroborating peers, summed reputation, best rank). Returns
    /// empty if nothing clears the bar — so a lone (possibly malicious) peer
    /// can't inject a result.
    pub(crate) fn consensus(&self, peer_hits: &[PeerHit], limit: usize) -> Vec<SearchResult> {
        let min_agree = self.cfg.min_agreement.max(1);
        struct Agg {
            result: SearchResult,
            peers: usize,
            rep_sum: f64,
            best_rank: usize,
        }
        let mut map: HashMap<String, Agg> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for ph in peer_hits {
            let mut seen: HashSet<String> = HashSet::new(); // one vote per peer per URL
            for (rank, r) in ph.hits.iter().enumerate() {
                let key = normalize_url(&r.url);
                if key.is_empty() || !seen.insert(key.clone()) {
                    continue;
                }
                let agg = map.entry(key.clone()).or_insert_with(|| {
                    order.push(key.clone());
                    Agg {
                        result: r.clone(),
                        peers: 0,
                        rep_sum: 0.0,
                        best_rank: usize::MAX,
                    }
                });
                agg.peers += 1;
                agg.rep_sum += ph.reputation;
                agg.best_rank = agg.best_rank.min(rank);
            }
        }
        let mut trusted: Vec<Agg> = order
            .into_iter()
            .filter_map(|k| map.remove(&k))
            .filter(|a| a.peers >= min_agree)
            .collect();
        trusted.sort_by(|a, b| {
            b.peers
                .cmp(&a.peers)
                .then(
                    b.rep_sum
                        .partial_cmp(&a.rep_sum)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(a.best_rank.cmp(&b.best_rank))
        });
        trusted
            .into_iter()
            .take(limit)
            .map(|mut a| {
                a.result.meta = Some(format!("hive: {} peers", a.peers));
                a.result
            })
            .collect()
    }

    /// Reward/penalize peers by how well their hits overlap a reference set (the
    /// consensus we returned, or the local truth). EMA toward the agreement ratio,
    /// so a peer that consistently disagrees decays toward irrelevance.
    pub(crate) fn update_reputations(&self, peer_hits: &[PeerHit], reference: &[SearchResult]) {
        if reference.is_empty() || peer_hits.is_empty() {
            return;
        }
        let reference: HashSet<String> = reference.iter().map(|r| normalize_url(&r.url)).collect();
        let mut peers = self.peers.lock().unwrap();
        for ph in peer_hits {
            let urls: HashSet<String> = ph.hits.iter().map(|r| normalize_url(&r.url)).collect();
            if urls.is_empty() {
                continue;
            }
            let overlap = urls.iter().filter(|u| reference.contains(*u)).count();
            let agreement = overlap as f64 / urls.len() as f64;
            if let Some(p) = peers.get_mut(&ph.url) {
                nudge_reputation(p, agreement);
            }
        }
    }

    /// Periodic background work: refresh peer digests and decay stale reputations.
    fn spawn_sync(self: Arc<Self>) {
        tokio::spawn(async move {
            let interval = Duration::from_secs(self.cfg.sync_secs.max(5));
            loop {
                self.sync_once().await;
                tokio::time::sleep(interval).await;
            }
        });
    }

    async fn sync_once(&self) {
        let urls: Vec<String> = self.peers.lock().unwrap().keys().cloned().collect();
        for url in urls {
            match fetch_digest(&self.http, &url, &self.cfg.token).await {
                Ok(d) if d.node_id != self.node_id && d.bloom.is_valid() => {
                    let mut peers = self.peers.lock().unwrap();
                    if let Some(p) = peers.get_mut(&url) {
                        p.bloom = Some(d.bloom);
                    }
                }
                Ok(_) => {
                    // Self or malformed: don't consult it.
                    let mut peers = self.peers.lock().unwrap();
                    if let Some(p) = peers.get_mut(&url) {
                        p.bloom = None;
                    }
                }
                Err(_) => {
                    // Unreachable: decay reputation toward neutral, drop stale bloom.
                    let mut peers = self.peers.lock().unwrap();
                    if let Some(p) = peers.get_mut(&url) {
                        p.reputation = 0.5 + (p.reputation - 0.5) * 0.8;
                        p.bloom = None;
                    }
                }
            }
        }
    }

    /// Start background tasks (digest sync + mDNS discovery). `bind_port` is the
    /// local HTTP port, used when advertising via mDNS.
    pub(crate) fn start(self: Arc<Self>, bind_port: u16) {
        if self.cfg.mdns {
            mdns::spawn(self.clone(), bind_port);
        }
        self.spawn_sync();
    }
}

/// Adjust one peer's reputation toward an observed agreement ratio (EMA).
fn nudge_reputation(peer: &mut Peer, agreement: f64) {
    const ALPHA: f64 = 0.3;
    peer.reputation = ((1.0 - ALPHA) * peer.reputation + ALPHA * agreement).clamp(0.0, 1.0);
}

async fn query_peer(
    http: &Client,
    base: &str,
    token: &str,
    key: &str,
    cap: usize,
) -> anyhow::Result<Vec<SearchResult>> {
    let mut req = http.post(format!("{base}/hive/query")).json(&QueryReq {
        key: key.to_string(),
    });
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(Vec::new());
    }
    let mut body: QueryResp = resp.json().await?;
    body.hits.truncate(cap);
    Ok(body.hits)
}

async fn fetch_digest(http: &Client, base: &str, token: &str) -> anyhow::Result<Digest> {
    let mut req = http.get(format!("{base}/hive/digest"));
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    Ok(req.send().await?.error_for_status()?.json().await?)
}

/// Normalize a peer base URL: trim, add scheme if missing, drop trailing slash.
fn normalize_base(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    if u.is_empty() {
        String::new()
    } else if u.starts_with("http://") || u.starts_with("https://") {
        u.to_string()
    } else {
        format!("http://{u}")
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// A best-effort unique node id derived from process start time + pid (not
/// security-sensitive — used to skip ourselves during discovery).
fn random_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seed = format!("{nanos}-{}", std::process::id());
    hash_key(&seed)[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hive_with(min_agreement: usize) -> Arc<Hive> {
        let cfg = NetworkConfig {
            enabled: true,
            min_agreement,
            ..NetworkConfig::default()
        };
        Hive::new(&cfg, Arc::new(TtlCache::new(60, 64)))
    }

    fn hit(url: &str) -> SearchResult {
        SearchResult {
            url: url.to_string(),
            title: url.to_string(),
            ..Default::default()
        }
    }

    fn peer(rep: f64, urls: &[&str]) -> PeerHit {
        PeerHit {
            url: String::new(),
            reputation: rep,
            hits: urls.iter().map(|u| hit(u)).collect(),
        }
    }

    #[test]
    fn consensus_requires_corroboration() {
        let hive = hive_with(2);
        let hits = vec![
            peer(0.8, &["https://a.com", "https://b.com"]),
            peer(0.7, &["https://a.com", "https://c.com"]),
        ];
        let out = hive.consensus(&hits, 10);
        // Only a.com is corroborated by 2 peers; b/c each have a single peer.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://a.com");
    }

    #[test]
    fn lone_peer_cannot_inject_results() {
        let hive = hive_with(2);
        // A single (possibly malicious) peer returns junk — nothing is trusted.
        let hits = vec![peer(
            0.9,
            &["https://evil.example/1", "https://evil.example/2"],
        )];
        assert!(hive.consensus(&hits, 10).is_empty());
    }

    #[test]
    fn min_agreement_one_trusts_any_peer() {
        let hive = hive_with(1);
        let hits = vec![peer(0.5, &["https://solo.example"])];
        let out = hive.consensus(&hits, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://solo.example");
    }
}
