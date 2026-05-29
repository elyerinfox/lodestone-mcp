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
//!
//! **File sharing** (`/hive/blob`): when the on-disk file store is enabled, the
//! digest's Bloom also advertises the store's entry hashes, and a peer can pull a
//! cached file's raw bytes by hash. This lets a PDF/file one node downloaded
//! (arXiv, IETF, …) be served from the mesh instead of every node re-hitting the
//! rate-limited source. Blobs are addressed by hash (the raw URL never crosses the
//! wire), served only if the `[network].token` matches, and carry no consensus — a
//! consumer that gets unusable bytes simply re-fetches from the authoritative source.

mod bloom;
mod mdns;

pub(crate) use bloom::{hash_bytes, hash_key};

use std::collections::{HashMap, HashSet, VecDeque};
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

/// What a node advertises: a Bloom filter of the hashes it currently has cached,
/// plus the peers it knows (for gossip-based mesh growth).
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Digest {
    pub node_id: String,
    pub generation: u64,
    pub count: usize,
    pub bloom: BloomFilter,
    #[serde(default)]
    pub peers: Vec<String>,
}

/// Max peer URLs advertised per digest, and the upper bound on the peer table —
/// keeps gossip from growing either without limit.
const MAX_GOSSIP_PEERS: usize = 64;
/// Drop a peer after this many consecutive failed digest fetches.
const MAX_PEER_MISSES: u32 = 5;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct QueryReq {
    pub key: String,
    /// Hops remaining for relay (0 = answer from our own cache only). `serde
    /// default` keeps older peers that send just `{key}` working.
    #[serde(default)]
    pub ttl: u32,
    /// Node ids already visited, to break relay loops.
    #[serde(default)]
    pub seen: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct QueryResp {
    pub hits: Vec<SearchResult>,
}

/// A request for a shared blob, addressed by its hash (never the raw key/URL —
/// only hashes cross the wire). Used by both `/hive/blob` and `/hive/blobinfo`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BlobReq {
    pub key: String,
}

/// `/hive/blobinfo` response: the **content hash** of a held blob (cheap — no
/// bytes), so a consumer can corroborate it across peers before trusting any bytes.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BlobInfo {
    pub hash: String,
    pub size: u64,
}

/// Per-blob seed accounting (BitTorrent-style): how much we've served to peers vs.
/// fetched from them. `ratio = served_bytes / fetched_bytes`.
#[derive(Debug, Default, Clone)]
pub(crate) struct BlobStat {
    pub served: u64,
    pub fetched: u64,
    pub served_bytes: u64,
    pub fetched_bytes: u64,
}

impl BlobStat {
    /// served/fetched bytes ratio; `None` until we've fetched it at least once.
    pub fn ratio(&self) -> Option<f64> {
        (self.fetched_bytes > 0).then(|| self.served_bytes as f64 / self.fetched_bytes as f64)
    }
}

/// One peer's response to a consult.
pub(crate) struct PeerHit {
    pub url: String,
    pub reputation: f64,
    pub hits: Vec<SearchResult>,
}

struct Peer {
    url: String,
    /// The peer's self-reported stable node id (machine-derived), once we've seen
    /// its digest. Used as a human-readable, machine-unique identity in reports.
    node_id: Option<String>,
    bloom: Option<BloomFilter>,
    reputation: f64,
    misses: u32,
    /// Peers this peer advertised (its neighbors) — forms the mesh graph.
    known: Vec<String>,
}

impl Peer {
    fn with_reputation(url: String, reputation: f64) -> Self {
        Self {
            url,
            node_id: None,
            bloom: None,
            reputation,
            misses: 0,
            known: Vec::new(),
        }
    }

    /// Reachable = we successfully fetched its digest recently (have its bloom).
    fn reachable(&self) -> bool {
        self.bloom.is_some()
    }
}

pub(crate) struct Hive {
    cfg: NetworkConfig,
    node_id: String,
    http: Client,
    cache: Arc<TtlCache>,
    /// Optional on-disk file store, shared over the mesh as raw bytes so a PDF/file
    /// one node fetched (arXiv, IETF, …) can be served to peers instead of every
    /// node re-hitting the rate-limited source.
    store: Option<Arc<crate::store::FileStore>>,
    /// Optional retrieval-output cache (page/PDF/doc text), also shared as blobs so
    /// work one node did isn't repeated by every node. All behind the digest Bloom.
    retrieval: Option<Arc<TtlCache>>,
    peers: Mutex<HashMap<String, Peer>>,
    /// Per-blob seed accounting (served vs. fetched), keyed by blob hash.
    seeds: Mutex<HashMap<String, BlobStat>>,
    /// Reputations loaded from `state_file` at startup; seeds peers as they appear.
    loaded_reps: HashMap<String, f64>,
}

impl Hive {
    /// Build the hive from config, the shared result cache, and (optionally) the
    /// file store whose bytes it will also share. Seeds the static peer list; mDNS
    /// (if enabled) adds more at runtime.
    pub(crate) fn new(
        cfg: &NetworkConfig,
        cache: Arc<TtlCache>,
        store: Option<Arc<crate::store::FileStore>>,
        retrieval: Option<Arc<TtlCache>>,
    ) -> Arc<Self> {
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
        let loaded_reps = load_reputations(&cfg.state_file);
        let mut peers = HashMap::new();
        for url in &cfg.peers {
            let u = normalize_base(url);
            if !u.is_empty() {
                let rep = loaded_reps.get(&u).copied().unwrap_or(0.5);
                peers.insert(u.clone(), Peer::with_reputation(u, rep));
            }
        }
        Arc::new(Self {
            cfg: cfg.clone(),
            node_id,
            http,
            cache,
            store,
            retrieval,
            peers: Mutex::new(peers),
            seeds: Mutex::new(HashMap::new()),
            loaded_reps,
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

    /// Build this node's digest: a Bloom filter over everything it can serve — the
    /// live search-cache keys **and** the file-store entry hashes — plus a bounded
    /// sample of known peers so neighbors can discover the wider mesh (gossip).
    pub(crate) async fn digest(&self) -> Digest {
        let mut keys = self.cache.keys();
        if let Some(store) = &self.store {
            keys.extend(store.hashes().await);
        }
        if let Some(ret) = &self.retrieval {
            keys.extend(ret.keys());
        }
        let peers: Vec<String> = {
            let table = self.peers.lock().unwrap();
            table.keys().take(MAX_GOSSIP_PEERS).cloned().collect()
        };
        Digest {
            node_id: self.node_id.clone(),
            generation: now_secs(),
            count: keys.len(),
            bloom: BloomFilter::from_keys(&keys),
            peers,
        }
    }

    /// Serve a shared blob by hash for the `/hive/blob` endpoint: a file-store entry
    /// (raw bytes), else a retrieval-cache entry (text as bytes). Both are keyed by
    /// the same hash space advertised in the digest Bloom.
    pub(crate) async fn blob_lookup(&self, key_hash: &str) -> Option<Vec<u8>> {
        if let Some(store) = &self.store {
            if let Some(bytes) = store.get_by_hash(key_hash).await {
                return Some(bytes);
            }
        }
        if let Some(ret) = &self.retrieval {
            if let Some(text) = ret.get(key_hash) {
                return Some(text.into_bytes());
            }
        }
        None
    }

    /// The content hash + size of a held blob (for `/hive/blobinfo`), so peers can
    /// corroborate *what* we'd serve before any bytes move.
    pub(crate) async fn blob_content_hash(&self, key_hash: &str) -> Option<BlobInfo> {
        let bytes = self.blob_lookup(key_hash).await?;
        Some(BlobInfo {
            hash: hash_bytes(&bytes),
            size: bytes.len() as u64,
        })
    }

    /// Record that we served `len` bytes of `key` to a peer (seed accounting).
    pub(crate) fn record_served(&self, key: &str, len: usize) {
        let mut m = self.seeds.lock().unwrap();
        let e = m.entry(key.to_string()).or_default();
        e.served += 1;
        e.served_bytes += len as u64;
    }

    fn record_fetched(&self, key: &str, len: usize) {
        let mut m = self.seeds.lock().unwrap();
        let e = m.entry(key.to_string()).or_default();
        e.fetched += 1;
        e.fetched_bytes += len as u64;
    }

    /// Seed accounting for one blob hash (served vs. fetched), if tracked.
    pub(crate) fn seed_for(&self, key_hash: &str) -> Option<BlobStat> {
        self.seeds.lock().unwrap().get(key_hash).cloned()
    }

    /// `consult_blob`, given a URL (hashed internally).
    pub(crate) async fn consult_blob(&self, url: &str) -> Option<Vec<u8>> {
        self.consult_blob_hash(&hash_key(url)).await
    }

    /// Pull a shared blob by hash, **anti-tamper**: corroborate first, verify last.
    ///   1. Ask Bloom-matching peers (rep-sorted) for the blob's *content hash*
    ///      (`/hive/blobinfo`, no bytes).
    ///   2. Trust only a content hash that `>= min_agreement` distinct peers agree
    ///      on — so a lone or malicious peer can't dictate the content. (With the
    ///      default `min_agreement = 2`, a single holder isn't trusted; raise
    ///      availability by lowering it to 1.)
    ///   3. Fetch the bytes from an agreeing peer and verify they hash to the agreed
    ///      value before accepting; otherwise the caller falls back to the source.
    pub(crate) async fn consult_blob_hash(&self, key: &str) -> Option<Vec<u8>> {
        let key = key.to_string();
        let mut targets: Vec<(String, f64)> = {
            let peers = self.peers.lock().unwrap();
            peers
                .values()
                .filter(|p| p.bloom.as_ref().is_some_and(|b| b.maybe_contains(&key)))
                .map(|p| (p.url.clone(), p.reputation))
                .collect()
        };
        if targets.is_empty() {
            return None;
        }
        targets.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        targets.truncate(self.cfg.max_peers.max(1));

        // 1. Gather each candidate's claimed content hash (cheap).
        let infos: Vec<(String, f64, String)> =
            futures::future::join_all(targets.iter().map(|(url, rep)| {
                let http = self.http.clone();
                let token = self.cfg.token.clone();
                let key = key.clone();
                let url = url.clone();
                let rep = *rep;
                async move {
                    blobinfo_peer(&http, &url, &token, &key)
                        .await
                        .map(|info| (url, rep, info.hash))
                }
            }))
            .await
            .into_iter()
            .flatten()
            .collect();
        if infos.is_empty() {
            return None;
        }

        // 2. Corroborate: a content hash must be agreed by >= min_agreement distinct
        //    peers (reputation breaks ties). A lone/disagreeing peer can't win.
        let min_agree = self.cfg.min_agreement.max(1);
        let mut tally: HashMap<String, (usize, f64)> = HashMap::new();
        for (_, rep, h) in &infos {
            let e = tally.entry(h.clone()).or_insert((0, 0.0));
            e.0 += 1;
            e.1 += *rep;
        }
        let agreed = tally
            .into_iter()
            .filter(|(_, (n, _))| *n >= min_agree)
            .max_by(|a, b| {
                a.1 .0.cmp(&b.1 .0).then(
                    a.1 .1
                        .partial_cmp(&b.1 .1)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
            })
            .map(|(h, _)| h)?;

        // 3. Fetch from an agreeing peer and verify the bytes hash to `agreed`.
        for (url, _, _) in infos.iter().filter(|(_, _, h)| *h == agreed) {
            if let Some(bytes) = blob_peer(&self.http, url, &self.cfg.token, &key).await {
                if !bytes.is_empty() && hash_bytes(&bytes) == agreed {
                    self.record_fetched(&key, bytes.len());
                    return Some(bytes);
                }
            }
        }
        None
    }

    /// A report of per-blob seed ratios (served vs. fetched bytes), newest-served
    /// first. Surfaced by the `hive_seeds` tool — BitTorrent-style: how much this
    /// node has given back to the mesh per file.
    pub(crate) fn seed_report(&self) -> String {
        let seeds = self.seeds.lock().unwrap();
        if seeds.is_empty() {
            return "No blobs served or fetched yet.".to_string();
        }
        let mut entries: Vec<(&String, &BlobStat)> = seeds.iter().collect();
        entries.sort_by(|a, b| b.1.served_bytes.cmp(&a.1.served_bytes));
        let (mut ts, mut tf) = (0u64, 0u64);
        let mut out = format!("Blob seed accounting ({} tracked):\n", entries.len());
        for (hash, s) in &entries {
            ts += s.served_bytes;
            tf += s.fetched_bytes;
            let ratio = s
                .ratio()
                .map(|r| format!("{r:.2}"))
                .unwrap_or_else(|| "∞".to_string());
            out.push_str(&format!(
                "\n  {hash}\n    served {}× ({}), fetched {}× ({}), ratio {ratio}",
                s.served,
                crate::util::human_size(s.served_bytes),
                s.fetched,
                crate::util::human_size(s.fetched_bytes),
            ));
        }
        let overall = if tf > 0 {
            format!("{:.2}", ts as f64 / tf as f64)
        } else {
            "∞".to_string()
        };
        out.push_str(&format!(
            "\n\nTotal: served {}, fetched {}, overall ratio {overall}",
            crate::util::human_size(ts),
            crate::util::human_size(tf),
        ));
        out
    }

    /// Answer a peer's query: our cached hits for `key_hash`, if any.
    pub(crate) fn local_lookup(&self, key_hash: &str) -> Vec<SearchResult> {
        self.cache
            .get(key_hash)
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    /// Register a peer discovered at runtime (mDNS or gossip). New peers inherit
    /// any persisted reputation; the table is capped so gossip can't grow it
    /// without bound. Never adds ourselves.
    pub(crate) fn add_peer(&self, url: &str) {
        let u = normalize_base(url);
        if u.is_empty() {
            return;
        }
        let mut peers = self.peers.lock().unwrap();
        if peers.contains_key(&u) || peers.len() >= MAX_GOSSIP_PEERS {
            return;
        }
        let rep = self.loaded_reps.get(&u).copied().unwrap_or(0.5);
        peers.insert(u.clone(), Peer::with_reputation(u, rep));
    }

    /// Number of known peers (test introspection).
    #[cfg(test)]
    pub(crate) fn peer_count(&self) -> usize {
        self.peers.lock().unwrap().len()
    }

    /// Answer an incoming `/hive/query`: serve from our own cache, else (while
    /// `ttl > 0` and we haven't been visited) relay to our bloom-matching peers
    /// one hop closer. The `seen` node-id set breaks loops.
    pub(crate) async fn answer_query(
        &self,
        key: &str,
        ttl: u32,
        seen: &[String],
    ) -> Vec<SearchResult> {
        if seen.iter().any(|id| id == &self.node_id) {
            return Vec::new(); // loop: already visited
        }
        let local = self.local_lookup(key);
        if !local.is_empty() || ttl == 0 {
            return local;
        }
        let mut seen2 = seen.to_vec();
        seen2.push(self.node_id.clone());
        self.forward(key, ttl - 1, &seen2).await
    }

    /// Query our bloom-matching peers (one hop) and merge their hits. Used by the
    /// relay path; each downstream peer is asked with the decremented ttl.
    async fn forward(&self, key: &str, ttl: u32, seen: &[String]) -> Vec<SearchResult> {
        let targets: Vec<String> = {
            let peers = self.peers.lock().unwrap();
            peers
                .values()
                .filter(|p| p.bloom.as_ref().is_some_and(|b| b.maybe_contains(key)))
                .map(|p| p.url.clone())
                .take(self.cfg.max_peers.max(1))
                .collect()
        };
        let cap = self.cfg.max_results_per_peer.max(1);
        let futs = targets.into_iter().map(|url| {
            let http = self.http.clone();
            let token = self.cfg.token.clone();
            let key = key.to_string();
            let seen = seen.to_vec();
            async move {
                query_peer(&http, &url, &token, &key, cap, ttl, &seen)
                    .await
                    .unwrap_or_default()
            }
        });
        // Merge downstream hits, deduped by normalized URL, capped.
        let mut merged: Vec<SearchResult> = Vec::new();
        let mut seen_urls: HashSet<String> = HashSet::new();
        for hits in futures::future::join_all(futs).await {
            for r in hits {
                if seen_urls.insert(normalize_url(&r.url)) {
                    merged.push(r);
                    if merged.len() >= cap {
                        return merged;
                    }
                }
            }
        }
        merged
    }

    /// Ask peers for `key_hash` in two passes:
    ///   * **Direct** — peers whose Bloom filter says they likely have it (ttl 0).
    ///   * **Relay** — when `relay_hops > 0`, reachable intermediaries whose own
    ///     Bloom doesn't match are asked to forward one+ hops toward a holder (so
    ///     a node can still reach data when it can't talk to the holder directly).
    ///
    /// Each top-level peer is exactly one consensus vote regardless of how it
    /// answered, so a relay can't fabricate corroboration. Bounded by `max_peers`,
    /// the per-request timeout, and capped result lists; `seen` stops loops.
    pub(crate) async fn consult(&self, key_hash: &str) -> Vec<PeerHit> {
        let max = self.cfg.max_peers.max(1);
        let mut direct: Vec<(String, f64)> = Vec::new();
        let mut relay: Vec<(String, f64)> = Vec::new();
        {
            let peers = self.peers.lock().unwrap();
            for p in peers.values() {
                match &p.bloom {
                    Some(b) if b.maybe_contains(key_hash) => {
                        direct.push((p.url.clone(), p.reputation))
                    }
                    Some(_) => relay.push((p.url.clone(), p.reputation)),
                    None => {}
                }
            }
        }
        direct.truncate(max);
        relay.truncate(max);

        let hops = self.cfg.relay_hops.min(2);
        let mut requests: Vec<(String, f64, u32)> =
            direct.into_iter().map(|(u, r)| (u, r, 0)).collect();
        if hops > 0 {
            requests.extend(relay.into_iter().map(|(u, r)| (u, r, hops)));
        }
        if requests.is_empty() {
            return Vec::new();
        }

        let cap = self.cfg.max_results_per_peer.max(1);
        let seen = vec![self.node_id.clone()]; // peers must not relay back to us
        let futs = requests.into_iter().map(|(url, reputation, ttl)| {
            let http = self.http.clone();
            let token = self.cfg.token.clone();
            let key = key_hash.to_string();
            let seen = seen.clone();
            async move {
                match query_peer(&http, &url, &token, &key, cap, ttl, &seen).await {
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
        let mut gossiped: Vec<String> = Vec::new();
        for url in urls {
            match fetch_digest(&self.http, &url, &self.cfg.token).await {
                Ok(d) if d.node_id != self.node_id && d.bloom.is_valid() => {
                    gossiped.extend(d.peers.iter().take(MAX_GOSSIP_PEERS).cloned());
                    let mut peers = self.peers.lock().unwrap();
                    if let Some(p) = peers.get_mut(&url) {
                        p.bloom = Some(d.bloom);
                        p.misses = 0;
                        p.known = d.peers;
                        p.node_id = Some(d.node_id);
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
                    // Unreachable: decay reputation toward neutral, drop stale bloom,
                    // and prune after too many consecutive misses (keeps gossiped or
                    // dead peers from accumulating).
                    let mut peers = self.peers.lock().unwrap();
                    if let Some(p) = peers.get_mut(&url) {
                        p.reputation = 0.5 + (p.reputation - 0.5) * 0.8;
                        p.bloom = None;
                        p.misses += 1;
                        if p.misses >= MAX_PEER_MISSES {
                            peers.remove(&url);
                        }
                    }
                }
            }
        }
        // Merge gossiped peers (skipping ourselves) into the table, then persist
        // reputations for the next restart.
        for u in gossiped {
            let u = normalize_base(&u);
            if !u.is_empty() {
                self.add_peer(&u);
            }
        }
        self.persist_reputations();
    }

    /// Persist current peer reputations to `state_file` (if configured) so trust
    /// survives restarts. Best-effort: write failures are logged, not fatal.
    fn persist_reputations(&self) {
        if self.cfg.state_file.is_empty() {
            return;
        }
        let map: HashMap<String, f64> = {
            let peers = self.peers.lock().unwrap();
            peers
                .values()
                .map(|p| (p.url.clone(), p.reputation))
                .collect()
        };
        match serde_json::to_string(&map) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.cfg.state_file, json) {
                    tracing::warn!(error = %e, path = %self.cfg.state_file, "hive: failed to persist reputations");
                }
            }
            Err(e) => tracing::warn!(error = %e, "hive: failed to serialize reputations"),
        }
    }

    /// A human-readable snapshot of the mesh: this node, each known peer's
    /// reputation/reachability, and the graph edges it advertised.
    pub(crate) fn graph_report(&self) -> String {
        let peers = self.peers.lock().unwrap();
        let mut out = format!(
            "Hivemind node {} — {} known peer(s):\n",
            self.node_id,
            peers.len()
        );
        let mut entries: Vec<&Peer> = peers.values().collect();
        entries.sort_by(|a, b| a.url.cmp(&b.url));
        for p in entries {
            let status = if p.reachable() {
                "reachable"
            } else {
                "unreachable"
            };
            out.push_str(&format!(
                "  {} [{status}] id {} reputation {:.2}{}\n",
                p.url,
                p.node_id.as_deref().unwrap_or("?"),
                p.reputation,
                if p.misses > 0 {
                    format!(" misses {}", p.misses)
                } else {
                    String::new()
                },
            ));
            if !p.known.is_empty() {
                out.push_str(&format!("      ↳ knows: {}\n", p.known.join(", ")));
            }
        }
        out
    }

    /// A report of each reachable node and **how many hops away** it is over the
    /// gossip graph (direct peers = 1 hop; nodes only reachable through a neighbor's
    /// advertised peer list are 2+). Direct peers also show their machine id,
    /// reputation, and reachability.
    pub(crate) fn peers_report(&self) -> String {
        let peers = self.peers.lock().unwrap();
        if peers.is_empty() {
            return format!("Hivemind node {} — no known peers.\n", self.node_id);
        }
        // BFS over the URL graph: self → direct peers (hop 1) → their `known` (hop 2+).
        let mut dist: HashMap<String, u32> = HashMap::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        for url in peers.keys() {
            dist.insert(url.clone(), 1);
            queue.push_back(url.clone());
        }
        while let Some(u) = queue.pop_front() {
            let d = dist[&u];
            if let Some(p) = peers.get(&u) {
                for n in &p.known {
                    let n = normalize_base(n);
                    if !n.is_empty() && !dist.contains_key(&n) {
                        dist.insert(n.clone(), d + 1);
                        queue.push_back(n);
                    }
                }
            }
        }
        let mut nodes: Vec<(String, u32)> = dist.into_iter().collect();
        nodes.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
        let mut out = format!(
            "Hivemind node {} — {} node(s) in reach:\n",
            self.node_id,
            nodes.len()
        );
        for (url, hops) in nodes {
            let hop_label = if hops == 1 { "hop" } else { "hops" };
            match peers.get(&url) {
                Some(p) => out.push_str(&format!(
                    "  {url}  ({hops} {hop_label}) id {} reputation {:.2} [{}]\n",
                    p.node_id.as_deref().unwrap_or("?"),
                    p.reputation,
                    if p.reachable() {
                        "reachable"
                    } else {
                        "unreachable"
                    },
                )),
                // A node only seen via a peer's advertised list (not a direct peer).
                None => out.push_str(&format!("  {url}  ({hops} {hop_label}) via gossip\n")),
            }
        }
        out
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

#[allow(clippy::too_many_arguments)]
async fn query_peer(
    http: &Client,
    base: &str,
    token: &str,
    key: &str,
    cap: usize,
    ttl: u32,
    seen: &[String],
) -> anyhow::Result<Vec<SearchResult>> {
    let mut req = http.post(format!("{base}/hive/query")).json(&QueryReq {
        key: key.to_string(),
        ttl,
        seen: seen.to_vec(),
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

/// Fetch a shared blob's raw bytes from one peer, or `None` if it doesn't have it.
async fn blob_peer(http: &Client, base: &str, token: &str, key: &str) -> Option<Vec<u8>> {
    let mut req = http.post(format!("{base}/hive/blob")).json(&BlobReq {
        key: key.to_string(),
    });
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
        return None;
    }
    resp.bytes().await.ok().map(|b| b.to_vec())
}

/// Ask one peer for a blob's content hash (cheap, no bytes), or `None`.
async fn blobinfo_peer(http: &Client, base: &str, token: &str, key: &str) -> Option<BlobInfo> {
    let mut req = http.post(format!("{base}/hive/blobinfo")).json(&BlobReq {
        key: key.to_string(),
    });
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
        return None;
    }
    resp.json().await.ok()
}

async fn fetch_digest(http: &Client, base: &str, token: &str) -> anyhow::Result<Digest> {
    let mut req = http.get(format!("{base}/hive/digest"));
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    Ok(req.send().await?.error_for_status()?.json().await?)
}

/// Load persisted peer reputations from `path` (JSON map url->reputation). Empty
/// map when disabled, missing, or malformed — never fatal.
fn load_reputations(path: &str) -> HashMap<String, f64> {
    if path.is_empty() {
        return HashMap::new();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
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

/// A **stable**, machine-derived default node id: the OS machine GUID (else the
/// hostname, else a random fallback), mixed with the bind port so two instances on
/// one host stay distinct yet each is stable across restarts. Hashed + truncated.
/// Used when `[network].node_id` isn't set explicitly.
pub(crate) fn default_node_id(bind: &str) -> String {
    let machine = machine_uid::get()
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(::sysinfo::System::host_name)
        .unwrap_or_else(random_id);
    let port = bind.rsplit(':').next().unwrap_or("");
    hash_key(&format!("{machine}:{port}"))[..16].to_string()
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
        Hive::new(&cfg, Arc::new(TtlCache::new(60, 64)), None, None)
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

    #[test]
    fn add_peer_dedupes_and_caps() {
        let hive = hive_with(2);
        hive.add_peer("http://a.example:8000");
        hive.add_peer("http://a.example:8000/"); // same after normalization
        assert_eq!(hive.peer_count(), 1);
        for i in 0..(MAX_GOSSIP_PEERS + 20) {
            hive.add_peer(&format!("http://peer{i}.example:8000"));
        }
        assert!(hive.peer_count() <= MAX_GOSSIP_PEERS);
    }

    #[test]
    fn reputation_persistence_round_trip() {
        let path = std::env::temp_dir().join(format!("lode-hive-{}.json", std::process::id()));
        let path_str = path.to_string_lossy().to_string();
        let cfg = NetworkConfig {
            enabled: true,
            peers: vec!["http://a.example:8000".into()],
            state_file: path_str.clone(),
            ..NetworkConfig::default()
        };
        let hive = Hive::new(&cfg, Arc::new(TtlCache::new(60, 64)), None, None);
        hive.persist_reputations();
        let loaded = load_reputations(&path_str);
        assert_eq!(loaded.get("http://a.example:8000").copied(), Some(0.5));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn answer_query_serves_local_and_guards_loops() {
        let hive = hive_with(2);
        let key = "abc123";
        let hits = vec![hit("https://x.example")];
        hive.cache
            .put(key.to_string(), serde_json::to_string(&hits).unwrap());

        // ttl 0, not yet visited → served from our cache.
        assert_eq!(hive.answer_query(key, 0, &[]).await.len(), 1);

        // Our own node id already in `seen` → loop guard returns nothing, even
        // though the entry is cached.
        let me = hive.node_id().to_string();
        assert!(hive.answer_query(key, 1, &[me]).await.is_empty());
    }

    #[test]
    fn blob_stat_ratio_and_content_hash() {
        let mut s = BlobStat::default();
        assert!(s.ratio().is_none()); // nothing fetched yet
        s.fetched_bytes = 100;
        s.served_bytes = 250;
        assert_eq!(s.ratio(), Some(2.5)); // served 2.5× what we fetched

        // Content hash is deterministic and payload-sensitive (tamper detection).
        assert_eq!(hash_bytes(b"hello"), hash_bytes(b"hello"));
        assert_ne!(hash_bytes(b"hello"), hash_bytes(b"hellp"));
    }

    #[test]
    fn records_seed_accounting() {
        let hive = hive_with(2);
        hive.record_fetched("k", 1000);
        hive.record_served("k", 1000);
        hive.record_served("k", 1000);
        let s = hive.seed_for("k").unwrap();
        assert_eq!((s.served, s.fetched), (2, 1));
        assert_eq!(s.ratio(), Some(2.0)); // gave back 2× what we took
    }
}
