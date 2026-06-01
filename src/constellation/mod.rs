//! The constellation — an opt-in peer-to-peer layer that lets instances consult each
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
//! **File sharing** (`/constellation/blob`): when the on-disk file store is enabled, the
//! digest's Bloom also advertises the store's entry hashes, and a peer can pull a
//! cached file's raw bytes by hash. This lets a PDF/file one node downloaded
//! (arXiv, IETF, …) be served from the mesh instead of every node re-hitting the
//! rate-limited source. Blobs are addressed by hash (the raw URL never crosses the
//! wire), served only if the `[network].token` matches, and carry no consensus — a
//! consumer that gets unusable bytes simply re-fetches from the authoritative source.

mod bloom;
pub mod delegation;
pub mod identifiers;
mod mdns;

pub(crate) use bloom::{hash_bytes, hash_key};
pub use identifiers::{Identifiers, Source};

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    /// The advertiser's shared constellation id (for convergence/merge). `serde
    /// default` keeps older peers (which omit it) working.
    #[serde(default)]
    pub constellation_id: String,
    pub generation: u64,
    pub count: usize,
    pub bloom: BloomFilter,
    #[serde(default)]
    pub peers: Vec<String>,
    /// `true` if this node opted into retrieval delegation — peers may POST
    /// `/constellation/retrieve` asking it to fetch a URL on their behalf,
    /// subject to its per-peer / per-job / per-hour rate limits.
    /// Older peers that omit this field land on `false` so requesters
    /// don't accidentally hammer a peer that hasn't advertised willingness.
    #[serde(default)]
    pub delegation_enabled: bool,
    /// Per-feature opt-in advertisement: query / retrieval / blob /
    /// browser. Used by `constellation_capabilities` and by the
    /// delegation paths to filter peers. Mirrors the local
    /// `[network].capabilities` config. `delegation_enabled` above is
    /// kept as a backward-compat alias for `capabilities.retrieval`.
    /// Older peers that omit this field land on the default
    /// (`query=true, retrieval=false, blob=true, browser=false`).
    #[serde(default)]
    pub capabilities: crate::config::Capabilities,
    /// **Full** count of reachable peers this node knows about, used as the
    /// primary signal in `maybe_adopt_id`: when two constellations meet, the
    /// **larger** mesh wins so the smaller mesh adopts the larger one's id
    /// (with the alphabetically-smaller id as a tiebreaker on equal sizes).
    ///
    /// Distinct from `peers` (which is a gossip *sample* capped at
    /// `MAX_GOSSIP_PEERS = 64`) and from `count` (which is the Bloom filter
    /// entry count). Older peers that omit this default to 0 — they'll lose
    /// every merge against newer peers, which is the safe default
    /// (they're either alone or upgrading).
    #[serde(default)]
    pub peer_count: usize,
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
/// only hashes cross the wire). Used by both `/constellation/blob` and `/constellation/blobinfo`.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BlobReq {
    pub key: String,
}

/// `/constellation/blobinfo` response: the **content hash** of a held blob (cheap — no
/// bytes), so a consumer can corroborate it across peers before trusting any bytes.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BlobInfo {
    pub hash: String,
    pub size: u64,
}

/// `POST /constellation/retrieve` — "go fetch this URL for me" delegation
/// request. The serving node performs the fetch, caches the body locally
/// (so this AND every peer benefits from the result), and streams it back.
/// `source` lets the requester carry the per-source policy hint over the
/// wire so the serving node caches with the right TTL.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RetrieveReq {
    /// The URL to fetch.
    pub url: String,
    /// Maximum body bytes the requester wants. Must be `<=
    /// [network].delegation_max_bytes_per_job` on the serving node.
    pub max_bytes: u64,
    /// The classifier the requester believes applies. Defaults to `Other`.
    #[serde(default)]
    pub source: identifiers::Source,
}

/// `POST /constellation/browser_persona` — "drive your browser session for
/// me" delegation (#128). The peer is asked to navigate its named persona
/// to a URL and return a compact observation. Sessions DO NOT
/// transport across the wire — each node maintains its own warm personas.
/// The peer's `capabilities.browser` must be ON or the request is
/// refused. The peer's local SSRF guard (#130) refuses any URL that
/// resolves to its local network.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BrowserPersonaReq {
    pub persona_name: String,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BrowserPersonaResp {
    pub url: String,
    pub title: String,
    /// Compact observation tree as the browser session manager produces
    /// it — list of interactive elements with stable selectors. Empty
    /// vec when nothing matched.
    pub tree: Vec<crate::skills::browser_session::TreeNode>,
}

/// `BrowserPersonaReq` rejection body. Same shape as `RetrieveReject` so
/// requesters can branch on the same `reason` field.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BrowserPersonaReject {
    pub reason: &'static str,
    pub message: String,
}

/// `RetrieveReq` rejection body (HTTP 429 / 400 / 403) — JSON payload telling
/// the requester *why* and how long to back off, so clients don't blindly
/// re-bombard a peer that's already at capacity.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RetrieveReject {
    /// Machine-readable reason: `"disabled"` / `"per_job_too_large"` /
    /// `"peer_jobs_exceeded"` / `"global_bytes_exceeded"` / `"fetch_failed"`.
    pub reason: &'static str,
    /// Suggested seconds to wait before retrying, or 0 if not retryable.
    #[serde(default)]
    pub retry_after_secs: u64,
    /// Human-readable detail (logged + shown in tracing breadcrumbs).
    #[serde(default)]
    pub detail: String,
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
    /// This peer advertised `delegation_enabled = true` on its most recent
    /// digest, so it'll accept `POST /constellation/retrieve`. `false` until
    /// we've seen a digest that says otherwise.
    delegation_enabled: bool,
    /// Per-feature opt-in set this peer advertised on its most recent
    /// digest. `None` means we haven't seen a digest yet (treat as the
    /// default — query+blob on, retrieval+browser off).
    capabilities: Option<crate::config::Capabilities>,
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
            delegation_enabled: false,
            capabilities: None,
        }
    }

    /// Reachable = we successfully fetched its digest recently (have its bloom).
    fn reachable(&self) -> bool {
        self.bloom.is_some()
    }
}

pub(crate) struct Constellation {
    cfg: NetworkConfig,
    node_id: String,
    /// The shared constellation id (distinct from `node_id`). Mutable so the mesh can
    /// converge to one id — co-located constellations merge by adopting the **larger**
    /// mesh's id (alphabetically smaller id as the tiebreaker on equal sizes).
    constellation_id: Mutex<String>,
    http: Client,
    cache: Arc<TtlCache>,
    /// Optional on-disk file store, shared over the mesh as raw bytes so a PDF/file
    /// one node fetched (arXiv, IETF, …) can be served to peers instead of every
    /// node re-hitting the rate-limited source.
    store: Option<Arc<crate::store::FileStore>>,
    /// Optional retrieval-output cache (page/PDF/doc text), also shared as blobs so
    /// work one node did isn't repeated by every node. All behind the digest Bloom.
    /// Multi-identifier: a single entry can be reached by primary key, URL aliases,
    /// source-specific ids, or content hash — see [`identifiers::Identifiers`].
    retrieval: Option<Arc<crate::retrieval::IndexedRetrievalCache>>,
    peers: Mutex<HashMap<String, Peer>>,
    /// Per-blob seed accounting (served vs. fetched), keyed by blob hash.
    seeds: Mutex<HashMap<String, BlobStat>>,
    /// Anti-storm: key-hash → when we last *relayed* it. The same query arriving via
    /// multiple paths (a dense or galaxy-linked mesh) is re-fanned only once per
    /// short window; duplicates fall back to a local-only answer.
    recent_relays: Mutex<HashMap<String, Instant>>,
    /// Reputations loaded from `state_file` at startup; seeds peers as they appear.
    loaded_reps: HashMap<String, f64>,
    /// Retrieval-delegation rate limiter. Active only when
    /// `[network].delegation_enabled = true`; the limiter itself is built
    /// regardless so its `Disabled` reason is the consistent rejection path.
    delegation: delegation::DelegationLimiter,
    /// URLs that *resolve to this very node* — populated at startup with
    /// `http://localhost:<port>` / `http://127.0.0.1:<port>` (and the IPv6
    /// equivalent), then extended dynamically when mDNS resolves our own
    /// service announcement so we learn every LAN-interface address mDNS
    /// advertised us on. `add_peer` checks this set before inserting so a
    /// peer that gossips our address back, an mDNS self-resolution that
    /// slips past the node-id dedup, or a misconfigured static peer entry
    /// can't accidentally make us our own peer.
    local_urls: Mutex<HashSet<String>>,
    /// Runtime-tunable overrides for a small subset of [network] knobs.
    /// Dashboard settings drawer writes here; reads inside the constellation
    /// consult these instead of the static `cfg` values. Ephemeral by
    /// design — never persisted, so a restart restores the config file's
    /// values. Knobs that require subsystem lifecycle changes (mdns,
    /// sync_secs, request_timeout_ms) live in `cfg` only; the UI shows
    /// them read-only as "restart required".
    runtime: Mutex<RuntimeOverrides>,
}

/// The subset of [network] knobs the dashboard can mutate without
/// restarting subsystems. Every read site inside the constellation
/// reads through this, not through `self.cfg`, when a runtime
/// override is allowed.
#[derive(Debug, Clone)]
pub(crate) struct RuntimeOverrides {
    pub delegation_enabled: bool,
    pub max_peers: usize,
    pub min_agreement: usize,
}

/// Sparse PATCH body — every field is optional so the dashboard can
/// send only the knob it actually changed. Anything outside the
/// allowed set lives on `cfg` and isn't representable here.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct RuntimeOverridesPatch {
    pub delegation_enabled: Option<bool>,
    pub max_peers: Option<usize>,
    pub min_agreement: Option<usize>,
}

impl Constellation {
    /// Build the constellation from config, the shared result cache, and (optionally) the
    /// file store whose bytes it will also share. Seeds the static peer list; mDNS
    /// (if enabled) adds more at runtime.
    pub(crate) fn new(
        cfg: &NetworkConfig,
        cache: Arc<TtlCache>,
        store: Option<Arc<crate::store::FileStore>>,
        retrieval: Option<Arc<crate::retrieval::IndexedRetrievalCache>>,
    ) -> Arc<Self> {
        let node_id = if cfg.node_id.trim().is_empty() {
            random_id()
        } else {
            cfg.node_id.trim().to_string()
        };
        // Shared constellation id: configured, else random. Convergence on sync makes
        // co-located nodes/meshes agree on a single id via `maybe_adopt_id` — the
        // larger mesh wins; alphabetical id is the tiebreaker on equal sizes.
        let constellation_id = if cfg.id.trim().is_empty() {
            random_id()
        } else {
            cfg.id.trim().to_string()
        };
        let http = Client::builder()
            .user_agent("lodestone-constellation")
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
        let delegation = delegation::DelegationLimiter::new(
            cfg.delegation_enabled,
            cfg.delegation_max_jobs_per_peer_per_hour,
            cfg.delegation_max_bytes_per_job,
            cfg.delegation_total_bytes_per_hour,
        );
        Arc::new(Self {
            cfg: cfg.clone(),
            node_id,
            constellation_id: Mutex::new(constellation_id),
            http,
            cache,
            store,
            retrieval,
            peers: Mutex::new(peers),
            seeds: Mutex::new(HashMap::new()),
            recent_relays: Mutex::new(HashMap::new()),
            loaded_reps,
            delegation,
            local_urls: Mutex::new(HashSet::new()),
            runtime: Mutex::new(RuntimeOverrides {
                delegation_enabled: cfg.delegation_enabled,
                max_peers: cfg.max_peers,
                min_agreement: cfg.min_agreement,
            }),
        })
    }

    /// Apply a sparse patch to runtime overrides. Fields the caller
    /// didn't set keep their current value. Returns the post-patch
    /// snapshot so the dashboard can confirm what stuck. Values are
    /// clamped to safe ranges so a typo in the dashboard can't disable
    /// the consensus check or starve the peer table.
    pub(crate) fn apply_runtime_patch(&self, patch: RuntimeOverridesPatch) -> RuntimeOverrides {
        let mut r = self.runtime.lock().unwrap();
        if let Some(v) = patch.delegation_enabled {
            r.delegation_enabled = v;
        }
        if let Some(v) = patch.max_peers {
            r.max_peers = v.clamp(1, 256);
        }
        if let Some(v) = patch.min_agreement {
            r.min_agreement = v.clamp(1, 16);
        }
        r.clone()
    }

    /// Effective runtime values — used at every hot-path read site
    /// instead of `self.cfg.*`, so dashboard edits take effect on the
    /// next call without a restart.
    fn delegation_enabled(&self) -> bool {
        self.runtime.lock().unwrap().delegation_enabled
    }
    fn max_peers(&self) -> usize {
        self.runtime.lock().unwrap().max_peers
    }
    fn min_agreement(&self) -> usize {
        self.runtime.lock().unwrap().min_agreement
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
        let (peers, peer_count): (Vec<String>, usize) = {
            let table = self.peers.lock().unwrap();
            let sample = table.keys().take(MAX_GOSSIP_PEERS).cloned().collect();
            // `peers` is a CAPPED gossip sample; `peer_count` is the FULL
            // count used by the merge rule so mesh size compares accurately
            // even when the gossip sample is saturated.
            (sample, table.len())
        };
        Digest {
            node_id: self.node_id.clone(),
            constellation_id: self.constellation_id.lock().unwrap().clone(),
            generation: now_secs(),
            count: keys.len(),
            delegation_enabled: self.delegation_enabled(),
            bloom: BloomFilter::from_keys(&keys),
            peers,
            peer_count,
            capabilities: self.effective_capabilities(),
        }
    }

    /// What this node currently advertises as its per-feature opt-in
    /// set. Reads through the runtime override for `retrieval` so the
    /// existing dashboard toggle for `delegation_enabled` keeps
    /// flipping the matching capability bit.
    pub(crate) fn effective_capabilities(&self) -> crate::config::Capabilities {
        crate::config::Capabilities {
            query: self.cfg.capabilities.query,
            // Runtime-tunable via the constellation settings drawer;
            // the static cfg value is the starting point.
            retrieval: self.delegation_enabled(),
            blob: self.cfg.capabilities.blob,
            browser: self.cfg.capabilities.browser,
        }
    }

    /// This node's current (possibly converged) constellation id.
    pub(crate) fn constellation_id(&self) -> String {
        self.constellation_id.lock().unwrap().clone()
    }

    /// Adopt `peer_cid` as our constellation id when meeting a peer in a
    /// different mesh. The rule is **larger mesh wins** with the
    /// alphabetically-smaller id as the tiebreaker on equal sizes:
    ///
    /// - `peer_peer_count > our peer count` → adopt (their mesh is bigger
    ///   so the smaller mesh — us — adopts the more-defined one);
    /// - equal counts AND `peer_cid < our_cid` → adopt (preserves
    ///   determinism — both ends compute the same answer);
    /// - everything else → no-op.
    ///
    /// Propagation is automatic via gossip: a node that adopts a new id
    /// re-advertises it on its next digest, peers see the change and run
    /// the same rule, and the new id spreads to every connected node in
    /// `O(sync_secs × mesh diameter)`.
    fn maybe_adopt_id(&self, peer_cid: &str, peer_peer_count: usize) {
        if peer_cid.is_empty() {
            return;
        }
        let my_peer_count = self.peers.lock().unwrap().len();
        let mut mine = self.constellation_id.lock().unwrap();
        // Larger mesh wins; alphabetical id is only the tiebreaker. An empty
        // local cid (uninitialised on first boot — shouldn't happen but be
        // defensive) always loses.
        let adopt = peer_peer_count > my_peer_count
            || (peer_peer_count == my_peer_count && (mine.is_empty() || peer_cid < mine.as_str()));
        if adopt {
            tracing::info!(
                adopted = %peer_cid,
                their_peers = peer_peer_count,
                our_peers = my_peer_count,
                "constellation id converged (merge — larger mesh wins)"
            );
            *mine = peer_cid.to_string();
        }
    }

    /// Serve a shared blob by hash for the `/constellation/blob` endpoint: a file-store entry
    /// (raw bytes), else a retrieval-cache entry (text as bytes). Both are keyed by
    /// the same hash space advertised in the digest Bloom.
    pub(crate) async fn blob_lookup(&self, key_hash: &str) -> Option<Vec<u8>> {
        if let Some(store) = &self.store {
            if let Some(bytes) = store.get_by_hash(key_hash).await {
                return Some(bytes);
            }
        }
        if let Some(ret) = &self.retrieval {
            if let Some(text) = ret.lookup_by_hash(key_hash) {
                return Some(text.into_bytes());
            }
        }
        None
    }

    /// The content hash + size of a held blob (for `/constellation/blobinfo`), so peers can
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

    /// Ask one constellation peer to fetch `url` on our behalf. Iterates
    /// reachable peers that advertised `delegation_enabled = true` on their
    /// most recent digest, sorted by reputation, and POSTs
    /// `/constellation/retrieve` to each until one accepts. Returns the
    /// fetched bytes on success, or `None` if every willing peer rejected
    /// (rate-limited, the fetch failed upstream, or no peers advertised
    /// delegation at all).
    ///
    /// `source` is forwarded over the wire so the serving node caches with
    /// the right per-source TTL — the entry then sits behind the existing
    /// Bloom-gated `consult_blob_hash` flow for everyone else in the mesh.
    /// `max_bytes` caps both how much the serving node is willing to
    /// download for us AND how much we'll accept back.
    ///
    /// The requester identifies itself via `X-Lodestone-Peer-Id` carrying
    /// `self.node_id`; the constellation `token` (if any) still gates who
    /// can request at all.
    pub(crate) async fn delegated_fetch(
        &self,
        url: &str,
        max_bytes: u64,
        source: identifiers::Source,
    ) -> Option<Vec<u8>> {
        // Snapshot the reachable, delegation-enabled peers (reputation-sorted).
        let mut targets: Vec<(String, f64)> = {
            let peers = self.peers.lock().unwrap();
            peers
                .values()
                .filter(|p| p.reachable() && p.delegation_enabled)
                .map(|p| (p.url.clone(), p.reputation))
                .collect()
        };
        if targets.is_empty() {
            return None;
        }
        targets.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        targets.truncate(self.max_peers().max(1));

        let req = RetrieveReq {
            url: url.to_string(),
            max_bytes,
            source,
        };
        for (peer_url, _rep) in targets {
            let mut post = self
                .http
                .post(format!("{peer_url}/constellation/retrieve"))
                .json(&req)
                .header("X-Lodestone-Peer-Id", &self.node_id);
            if !self.cfg.token.is_empty() {
                post = post.bearer_auth(&self.cfg.token);
            }
            let resp = match post.send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!(peer = %peer_url, error = %e, "delegated_fetch peer unreachable");
                    continue;
                }
            };
            let status = resp.status();
            if status.is_success() {
                match resp.bytes().await {
                    Ok(b) if !b.is_empty() && (b.len() as u64) <= max_bytes => {
                        self.record_fetched(url, b.len());
                        return Some(b.to_vec());
                    }
                    Ok(b) => {
                        tracing::debug!(
                            peer = %peer_url,
                            len = b.len(),
                            "delegated_fetch peer returned empty or oversized body"
                        );
                    }
                    Err(e) => {
                        tracing::debug!(peer = %peer_url, error = %e, "delegated_fetch body read failed");
                    }
                }
            } else {
                // 429 / 403 / 502 — log the reason so an operator running
                // `[tracing] = debug` can see which peers refused and why.
                let body = resp.text().await.unwrap_or_default();
                tracing::debug!(
                    peer = %peer_url,
                    status = %status,
                    reason = %body,
                    "delegated_fetch peer refused"
                );
            }
        }
        None
    }

    /// Serve a delegated `POST /constellation/retrieve` request:
    /// 1. Reserve a delegation slot on the rate limiter (rejects if
    ///    delegation is disabled, this peer is over its per-hour job
    ///    quota, the per-job byte cap would be exceeded, or the global
    ///    hourly byte budget is saturated).
    /// 2. Fetch the URL using the constellation's HTTP client. Honors the
    ///    per-job byte cap by capping the response read at `max_bytes`.
    /// 3. Cache the body in [`crate::retrieval::IndexedRetrievalCache`]
    ///    under the requester-supplied [`identifiers::Source`] so this
    ///    node, the requester, and every other peer in the mesh can
    ///    serve it via the existing `consult_blob_hash` path after.
    /// 4. Commit the limiter slot with the actual byte count and return
    ///    the bytes.
    ///
    /// On `Err(RetrieveReject)` no fetch happens and no cache entry is
    /// produced. The `Reject` carries a machine-readable reason + a
    /// suggested Retry-After hint so the requester can back off
    /// intelligently or try a different peer.
    pub(crate) async fn serve_retrieve(
        &self,
        peer_id: &str,
        req: &RetrieveReq,
    ) -> Result<Vec<u8>, RetrieveReject> {
        use delegation::RejectReason;
        // Reservation comes first — saturated peers don't get to spend our
        // egress trying.
        let slot = match self.delegation.try_acquire(peer_id, req.max_bytes) {
            Ok(slot) => slot,
            Err(RejectReason::Disabled) => {
                return Err(RetrieveReject {
                    reason: "disabled",
                    retry_after_secs: 0,
                    detail: "this node has [network].delegation_enabled = false".to_string(),
                });
            }
            Err(RejectReason::PerJobBytesExceeded { limit, requested }) => {
                return Err(RetrieveReject {
                    reason: "per_job_too_large",
                    retry_after_secs: 0,
                    detail: format!("requested {requested} bytes; cap is {limit}"),
                });
            }
            Err(RejectReason::PeerJobsExceeded { retry_after_secs }) => {
                return Err(RetrieveReject {
                    reason: "peer_jobs_exceeded",
                    retry_after_secs,
                    detail: "your peer has hit its hourly delegation quota".to_string(),
                });
            }
            Err(RejectReason::GlobalBytesExceeded { retry_after_secs }) => {
                return Err(RetrieveReject {
                    reason: "global_bytes_exceeded",
                    retry_after_secs,
                    detail: "this node's hourly delegation byte budget is saturated".to_string(),
                });
            }
        };

        // Fetch from upstream. The slot is held throughout; on any error
        // path we just let it drop, rolling the reservation back so a
        // bad URL doesn't burn the requester's quota.
        let resp = match self.http.get(&req.url).send().await {
            Ok(r) => r,
            Err(e) => {
                return Err(RetrieveReject {
                    reason: "fetch_failed",
                    retry_after_secs: 0,
                    detail: format!("upstream fetch failed: {e}"),
                });
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(RetrieveReject {
                reason: "fetch_failed",
                retry_after_secs: 0,
                detail: format!("upstream returned {status}"),
            });
        }
        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return Err(RetrieveReject {
                    reason: "fetch_failed",
                    retry_after_secs: 0,
                    detail: format!("body read failed: {e}"),
                });
            }
        };

        // Enforce the per-job byte cap (upstream might have ignored
        // Content-Length / Range hints).
        if bytes.len() as u64 > req.max_bytes {
            return Err(RetrieveReject {
                reason: "per_job_too_large",
                retry_after_secs: 0,
                detail: format!(
                    "upstream returned {} bytes; cap is {}",
                    bytes.len(),
                    req.max_bytes
                ),
            });
        }

        // Cache + commit. The cache write uses the same identifier path
        // that local retrieval uses, so the entry is reachable via the
        // existing `consult_blob_hash` flow with no additional plumbing.
        if let Some(ret) = &self.retrieval {
            // We need a text body for the IndexedRetrievalCache; bytes
            // that aren't valid UTF-8 lose information here but we keep
            // them in the file store path (a separate concern). The
            // delegated-fetch path is primarily for page text / JSON /
            // PDF-extracted text, all UTF-8-safe.
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let ids = identifiers::Identifiers::new(format!("delegated|{}", req.url))
                .with_source(req.source)
                .with_url(&req.url);
            ret.put(&ids, &text);
        }
        let actual = bytes.len() as u64;
        slot.commit(actual);
        Ok(bytes.to_vec())
    }

    /// `consult_blob`, given a URL (hashed internally). No source hint — uses
    /// the global `min_agreement`. Callers that have an `Identifiers` should
    /// reach for [`Self::consult_blob_hash_sourced`] so per-source policy
    /// applies.
    pub(crate) async fn consult_blob(&self, url: &str) -> Option<Vec<u8>> {
        self.consult_blob_hash(&hash_key(url)).await
    }

    /// Equivalent to [`Self::consult_blob_hash_sourced`] with
    /// `Source::Other` — no per-source policy relaxation; the global
    /// `min_agreement` floor applies. Existing call sites land here.
    pub(crate) async fn consult_blob_hash(&self, key: &str) -> Option<Vec<u8>> {
        self.consult_blob_hash_sourced(key, identifiers::Source::Other)
            .await
    }

    /// Pull a shared blob by hash, **anti-tamper**: corroborate first, verify last.
    ///   1. Ask Bloom-matching peers (rep-sorted) for the blob's *content hash*
    ///      (`/constellation/blobinfo`, no bytes).
    ///   2. Trust only a content hash that `>= min_agreement` distinct peers agree
    ///      on — so a lone or malicious peer can't dictate the content. (With the
    ///      default `min_agreement = 2`, a single holder isn't trusted; raise
    ///      availability by lowering it to 1.)
    ///   3. Fetch the bytes from an agreeing peer and verify they hash to the agreed
    ///      value before accepting; otherwise the caller falls back to the source.
    ///
    /// `source` is a per-call hint of the upstream the consumer believes the
    /// hash belongs to, and it changes what counts as "agreed":
    /// - **Content-addressable** (Wayback, arXiv by `id+v`, GitHub release
    ///   by tag) → a single peer suffices. The consumer was asking by a
    ///   hash derived from the source-specific identifier *and* the
    ///   consult-bytes step verifies the bytes hash to the agreed value,
    ///   so a malicious peer can't substitute content without producing a
    ///   different hash than the one the consumer was looking up by. The
    ///   global `cfg.min_agreement` is intentionally **not** honored here
    ///   — the safety isn't coming from peer consensus, it's coming from
    ///   the consumer's hash check, so a 3-peer requirement would only add
    ///   latency without adding safety. This is what makes long-tail
    ///   rate-limited content actually usable across the mesh.
    /// - **Volatile / non-content-addressable** (Overpass, search engines,
    ///   `Other`) → multi-peer corroboration matters because the consumer
    ///   has nothing to verify the bytes *against*. The effective floor is
    ///   `max(cfg.min_agreement, source.min_agreement_floor())`, so a user
    ///   that hardens to `min_agreement = 3` is never silently relaxed.
    pub(crate) async fn consult_blob_hash_sourced(
        &self,
        key: &str,
        source: identifiers::Source,
    ) -> Option<Vec<u8>> {
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
        targets.truncate(self.max_peers().max(1));

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

        // 2. Corroborate: a content hash must be agreed by >= min_agreement
        //    distinct peers. Per-source policy: content-addressable upstreams
        //    use the source floor (typically 1) because the consumer's
        //    bytes-hash check is the primary safety; everything else takes
        //    max(cfg, source floor) so users can harden upward.
        let min_agree = match source {
            identifiers::Source::Wayback
            | identifiers::Source::Arxiv
            | identifiers::Source::Github => source.min_agreement_floor(),
            _ => self.min_agreement().max(source.min_agreement_floor()),
        }
        .max(1);
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
    /// first. Surfaced by the `constellation_seeds` tool — BitTorrent-style: how much this
    /// node has given back to the mesh per file.
    pub(crate) fn seed_report(&self) -> String {
        let seeds = self.seeds.lock().unwrap();
        if seeds.is_empty() {
            return "No blobs served or fetched yet.".to_string();
        }
        let mut entries: Vec<(&String, &BlobStat)> = seeds.iter().collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.1.served_bytes));
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
        // Refuse to add ourselves. Covers every discovery path: mDNS
        // self-resolution that slips past the node-id dedup, gossip that
        // carries our address back, or a misconfigured static peer entry.
        // `local_urls` is seeded with localhost variants at startup and
        // extended with each LAN-interface address as mDNS resolves us.
        if self.local_urls.lock().unwrap().contains(&u) {
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

    /// Number of peers currently in the table (static + discovered). Used by the
    /// galaxy client to tell when local discovery has produced a constellation.
    pub(crate) fn known_peer_count(&self) -> usize {
        self.peers.lock().unwrap().len()
    }

    /// Build a privacy-safe snapshot of the constellation state for the
    /// dashboard WebSocket feed. Contains no secrets — never the cluster
    /// token, never the request body of any cached entry, never any peer
    /// auth material. Per-peer rows carry just URL + node_id + reputation
    /// + reachability + advertised-delegation flag.
    pub(crate) fn ws_state(&self) -> crate::ws::ConstellationState {
        let peers: Vec<crate::ws::PeerEntry> = {
            let table = self.peers.lock().unwrap();
            table
                .values()
                .map(|p| crate::ws::PeerEntry {
                    url: p.url.clone(),
                    node_id: p.node_id.clone(),
                    reputation: p.reputation,
                    reachable: p.reachable(),
                    delegation_enabled: p.delegation_enabled,
                    known_peers: p.known.clone(),
                    capabilities: p.capabilities.clone(),
                })
                .collect()
        };
        let local_urls: Vec<String> = {
            let mut v: Vec<String> =
                self.local_urls.lock().unwrap().iter().cloned().collect();
            v.sort();
            v
        };
        let (served, fetched) = {
            let seeds = self.seeds.lock().unwrap();
            seeds.values().fold((0u64, 0u64), |(s, f), st| {
                (s + st.served_bytes, f + st.fetched_bytes)
            })
        };
        crate::ws::ConstellationState {
            enabled: true,
            node_id: self.node_id.clone(),
            constellation_id: self.constellation_id.lock().unwrap().clone(),
            peer_count: peers.len(),
            peers,
            delegation_enabled: self.delegation_enabled(),
            delegation_max_jobs_per_peer_per_hour: self.cfg.delegation_max_jobs_per_peer_per_hour,
            delegation_max_bytes_per_job: self.cfg.delegation_max_bytes_per_job,
            delegation_total_bytes_per_hour: self.cfg.delegation_total_bytes_per_hour,
            total_served_bytes: served,
            total_fetched_bytes: fetched,
            local_urls,
            max_peers: self.max_peers(),
            min_agreement: self.min_agreement(),
            mdns_configured: self.cfg.mdns,
            sync_secs_configured: self.cfg.sync_secs,
            request_timeout_ms_configured: self.cfg.request_timeout_ms,
            local_capabilities: self.effective_capabilities(),
        }
    }

    /// Inbound handler for `POST /constellation/browser_persona`. Refuses
    /// the request if `[network].capabilities.browser` is false (the
    /// node hasn't opted in to delegated browser work). Otherwise,
    /// fetches-or-creates a persona isolated by the requesting peer's
    /// node id — `delegated:<peer_id>:<name>` — so peers A and B
    /// don't share cookies on the same logical persona name. The session
    /// is SSRF-guarded (#130). Navigates and returns the compact
    /// observation tree.
    pub(crate) async fn answer_browser_persona(
        &self,
        peer_id: &str,
        req: &BrowserPersonaReq,
    ) -> Result<BrowserPersonaResp, BrowserPersonaReject> {
        if !self.cfg.capabilities.browser {
            return Err(BrowserPersonaReject {
                reason: "disabled",
                message: "this node hasn't opted in to delegated browser work \
                          ([network.capabilities].browser = false)"
                    .to_string(),
            });
        }
        let mgr = crate::skills::browser_session::manager().await;
        let (session_id, _state) = mgr
            .persona_get_for_peer(peer_id, &req.persona_name)
            .await
            .map_err(|e| BrowserPersonaReject {
                reason: "persona_unavailable",
                message: format!("{e:?}"),
            })?;
        if let Err(e) = mgr.navigate(&session_id, &req.url).await {
            return Err(BrowserPersonaReject {
                reason: "navigate_failed",
                message: format!("{e:?}"),
            });
        }
        let obs = mgr
            .observe(
                &session_id,
                crate::skills::browser_session::ObserveMode::Tree,
            )
            .await
            .unwrap_or_default();
        let url = mgr
            .session_url(&session_id)
            .await
            .unwrap_or_else(|| req.url.clone());
        let title = mgr
            .session_title(&session_id)
            .await
            .unwrap_or_default();
        Ok(BrowserPersonaResp {
            url,
            title,
            tree: obs.tree.unwrap_or_default(),
        })
    }

    /// Outbound delegator: pick a peer whose advertised capability set
    /// includes `browser`, POST our request, return the response. The
    /// candidate list is shuffled-by-reputation so a busy peer doesn't
    /// always get picked first. Returns the first successful response;
    /// on every-peer-failure, wraps the last error.
    pub(crate) async fn delegate_browser_persona(
        &self,
        req: BrowserPersonaReq,
    ) -> Result<BrowserPersonaResp, String> {
        let candidates = self.peers_with_capability("browser");
        if candidates.is_empty() {
            return Err("no peer in the constellation has capabilities.browser = true; \
                        the local request is the only option"
                .to_string());
        }
        let timeout = std::time::Duration::from_millis(self.cfg.request_timeout_ms * 4);
        let mut last_err = String::from("no peer responded");
        for peer_url in candidates {
            let mut rq = self
                .http
                .post(format!("{peer_url}/constellation/browser_persona"))
                .header("x-lodestone-peer-id", &self.node_id)
                .json(&req)
                .timeout(timeout);
            if !self.cfg.token.is_empty() {
                rq = rq.bearer_auth(&self.cfg.token);
            }
            match rq.send().await {
                Ok(resp) if resp.status().is_success() => match resp.json::<BrowserPersonaResp>().await
                {
                    Ok(body) => return Ok(body),
                    Err(e) => {
                        last_err = format!("{peer_url}: invalid response body: {e}");
                    }
                },
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    last_err = format!("{peer_url}: {status} {body}");
                }
                Err(e) => {
                    last_err = format!("{peer_url}: {e}");
                }
            }
        }
        Err(last_err)
    }

    /// Read-only view of every known peer's currently-advertised
    /// capability set. Powers the `constellation_capabilities` tool
    /// and the outbound-delegation filter. Each row is
    /// `(node_id_or_url, capabilities, reachable)`. `capabilities` is
    /// `None` until we've successfully fetched the peer's digest at
    /// least once.
    pub(crate) fn peer_capability_view(
        &self,
    ) -> Vec<(String, Option<crate::config::Capabilities>, bool)> {
        let table = self.peers.lock().unwrap();
        table
            .values()
            .map(|p| {
                let label = p.node_id.clone().unwrap_or_else(|| p.url.clone());
                (label, p.capabilities.clone(), p.reachable())
            })
            .collect()
    }

    /// Return the URLs of reachable peers whose advertised capability
    /// set has `cap` enabled. Used by the outbound-delegation paths
    /// (#128) to filter peer candidates so we never ask a peer to do
    /// something it didn't opt into.
    pub(crate) fn peers_with_capability(&self, cap: &str) -> Vec<String> {
        let table = self.peers.lock().unwrap();
        table
            .values()
            .filter(|p| p.reachable())
            .filter(|p| match (cap, p.capabilities.as_ref()) {
                ("query", Some(c)) => c.query,
                ("retrieval", Some(c)) => c.retrieval,
                ("blob", Some(c)) => c.blob,
                ("browser", Some(c)) => c.browser,
                // No digest yet — treat conservatively as "not opted
                // in" for everything except query/blob, which default
                // to true.
                ("query", None) => true,
                ("blob", None) => true,
                _ => false,
            })
            .map(|p| p.url.clone())
            .collect()
    }

    /// Answer an incoming `/constellation/query`: serve from our own cache, else (while
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
        // Storm guard: the `seen` set only covers nodes on *this* path, so the same
        // query reaching us via several paths would otherwise be re-fanned each time.
        // Relay a given key at most once per short window; duplicates answer locally
        // (here, empty) instead of amplifying. Direct cache hits above are unaffected.
        if !self.should_relay(key) {
            return local;
        }
        let mut seen2 = seen.to_vec();
        seen2.push(self.node_id.clone());
        self.forward(key, ttl - 1, &seen2).await
    }

    /// True at most once per `key` within a short window (then false until it
    /// expires). Gates only the amplifying *relay* fan-out — see [`answer_query`].
    fn should_relay(&self, key: &str) -> bool {
        const WINDOW: Duration = Duration::from_secs(10);
        let now = Instant::now();
        let mut recent = self.recent_relays.lock().unwrap();
        recent.retain(|_, t| now.duration_since(*t) < WINDOW);
        if recent.contains_key(key) {
            return false;
        }
        recent.insert(key.to_string(), now);
        true
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
                .take(self.max_peers().max(1))
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
        let max = self.max_peers().max(1);
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
    ///
    /// Search results are inherently `Source::SearchEngine` — volatile,
    /// non-content-addressable — so the effective floor is
    /// `max(cfg.min_agreement, SearchEngine.min_agreement_floor())` = at
    /// least 2. A user that relaxes to `min_agreement = 1` for some other
    /// reason doesn't accidentally accept lone-peer search results.
    pub(crate) fn consensus(&self, peer_hits: &[PeerHit], limit: usize) -> Vec<SearchResult> {
        let min_agree = self
            .cfg
            .min_agreement
            .max(identifiers::Source::SearchEngine.min_agreement_floor())
            .max(1);
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
                a.result.meta = Some(format!("constellation: {} peers", a.peers));
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
                    let peer_cid = d.constellation_id.clone();
                    let peer_delegation = d.delegation_enabled;
                    let peer_peer_count = d.peer_count;
                    let peer_caps = d.capabilities.clone();
                    gossiped.extend(d.peers.iter().take(MAX_GOSSIP_PEERS).cloned());
                    {
                        let mut peers = self.peers.lock().unwrap();
                        if let Some(p) = peers.get_mut(&url) {
                            p.bloom = Some(d.bloom);
                            p.misses = 0;
                            p.known = d.peers;
                            p.node_id = Some(d.node_id);
                            p.delegation_enabled = peer_delegation;
                            p.capabilities = Some(peer_caps);
                        }
                    }
                    // Merge to the LARGER mesh; alphabetical id is only a
                    // tiebreaker. Each adopt propagates to our own peers on
                    // the next digest exchange so a connected mesh converges
                    // in O(sync_secs × diameter).
                    self.maybe_adopt_id(&peer_cid, peer_peer_count);
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
                    // dead peers from accumulating). Capture the departing peer's
                    // node_id BEFORE dropping so we can evict any delegated
                    // browser personas it left behind.
                    let evicted_node_id = {
                        let mut peers = self.peers.lock().unwrap();
                        if let Some(p) = peers.get_mut(&url) {
                            p.reputation = 0.5 + (p.reputation - 0.5) * 0.8;
                            p.bloom = None;
                            p.misses += 1;
                            if p.misses >= MAX_PEER_MISSES {
                                let id = p.node_id.clone();
                                peers.remove(&url);
                                id
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };
                    if let Some(node_id) = evicted_node_id {
                        // Tear down browser personas the departing peer
                        // owned. Lazy: only does work if the browser
                        // session manager has been initialized AND
                        // some delegated persona actually matches the id.
                        if let Some(mgr) =
                            crate::skills::browser_session::manager_if_init()
                        {
                            let dropped = mgr.evict_personas_for_peer(&node_id).await;
                            if dropped > 0 {
                                tracing::info!(
                                    peer_node_id = %node_id,
                                    personas = dropped,
                                    "peer departed — evicted its delegated browser personas",
                                );
                            }
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
                    tracing::warn!(error = %e, path = %self.cfg.state_file, "constellation: failed to persist reputations");
                }
            }
            Err(e) => tracing::warn!(error = %e, "constellation: failed to serialize reputations"),
        }
    }

    /// A human-readable snapshot of the mesh: this node, each known peer's
    /// reputation/reachability, and the graph edges it advertised.
    pub(crate) fn graph_report(&self) -> String {
        let peers = self.peers.lock().unwrap();
        let mut out = format!(
            "Constellation node {} — {} known peer(s):\n",
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
    /// Human-readable capability matrix: this node, then each peer
    /// we've seen a digest from, with a row of ON/OFF flags for every
    /// capability. `cap_filter` (when set) hides rows where the named
    /// capability is OFF — answering "who can do X?".
    pub(crate) fn capabilities_report(&self, cap_filter: Option<&str>) -> String {
        use std::fmt::Write as _;
        let own = self.effective_capabilities();
        let mut out = String::new();
        let _ = writeln!(
            out,
            "Constellation node {}  (constellation_id={})\n",
            self.node_id,
            self.constellation_id.lock().unwrap()
        );
        let _ = writeln!(out, "Capabilities a column shows:");
        let _ = writeln!(
            out,
            "  query     — answer cache consults\n  retrieval — fetch URLs for peers \
             (POST /constellation/retrieve)\n  blob      — serve file-store blobs to peers\n  \
             browser   — accept delegated browser actions"
        );
        let _ = writeln!(out);
        fn yn(b: bool) -> &'static str {
            if b { "ON" } else { "off" }
        }
        let cap_str = |c: &crate::config::Capabilities| -> String {
            format!(
                "query={} retrieval={} blob={} browser={}",
                yn(c.query),
                yn(c.retrieval),
                yn(c.blob),
                yn(c.browser),
            )
        };
        let cap_match = |c: &crate::config::Capabilities| -> bool {
            match cap_filter {
                Some("query") => c.query,
                Some("retrieval") => c.retrieval,
                Some("blob") => c.blob,
                Some("browser") => c.browser,
                _ => true,
            }
        };
        if cap_match(&own) {
            let _ = writeln!(out, "  [self]              {}", cap_str(&own));
        }
        let peers = self.peers.lock().unwrap();
        let mut rows: Vec<_> = peers.values().collect();
        rows.sort_by(|a, b| a.url.cmp(&b.url));
        for p in rows {
            let label = p.node_id.as_deref().unwrap_or(p.url.as_str());
            match &p.capabilities {
                Some(c) if cap_match(c) => {
                    let _ = writeln!(out, "  {label:<20} {}", cap_str(c));
                }
                None if cap_filter.is_none() => {
                    let _ = writeln!(out, "  {label:<20} (digest not yet seen)");
                }
                _ => {}
            }
        }
        out
    }

    pub(crate) fn peers_report(&self) -> String {
        let peers = self.peers.lock().unwrap();
        if peers.is_empty() {
            return format!("Constellation node {} — no known peers.\n", self.node_id);
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
            "Constellation node {} — {} node(s) in reach:\n",
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
        // Seed the self-URL set with the loopback addresses our
        // advertised port resolves through. mDNS will add the LAN
        // addresses dynamically as it resolves our own service.
        let port = self.advertise_port(bind_port);
        {
            let mut set = self.local_urls.lock().unwrap();
            for url in [
                format!("http://localhost:{port}"),
                format!("http://127.0.0.1:{port}"),
                format!("http://[::1]:{port}"),
            ] {
                let n = normalize_base(&url);
                if !n.is_empty() {
                    set.insert(n);
                }
            }
        }
        if self.cfg.mdns {
            mdns::spawn(self.clone(), bind_port);
        }
        self.spawn_sync();
    }

    /// Record a URL that resolves to this node so future `add_peer` calls
    /// skip it. Called from the mDNS resolution loop when our own service
    /// announcement comes back — every LAN-interface address mDNS chose
    /// to advertise us on lands here, so a peer that gossips any of
    /// those addresses back can't accidentally make us our own peer.
    pub(crate) fn mark_local_url(&self, url: &str) {
        let n = normalize_base(url);
        if !n.is_empty() {
            self.local_urls.lock().unwrap().insert(n);
        }
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
    let mut req = http
        .post(format!("{base}/constellation/query"))
        .json(&QueryReq {
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
    let mut req = http
        .post(format!("{base}/constellation/blob"))
        .json(&BlobReq {
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
    let mut req = http
        .post(format!("{base}/constellation/blobinfo"))
        .json(&BlobReq {
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
    let mut req = http.get(format!("{base}/constellation/digest"));
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

    fn constellation_with(min_agreement: usize) -> Arc<Constellation> {
        let cfg = NetworkConfig {
            enabled: true,
            min_agreement,
            ..NetworkConfig::default()
        };
        Constellation::new(&cfg, Arc::new(TtlCache::new(60, 64)), None, None)
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
        let constellation = constellation_with(2);
        let hits = vec![
            peer(0.8, &["https://a.com", "https://b.com"]),
            peer(0.7, &["https://a.com", "https://c.com"]),
        ];
        let out = constellation.consensus(&hits, 10);
        // Only a.com is corroborated by 2 peers; b/c each have a single peer.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://a.com");
    }

    #[test]
    fn lone_peer_cannot_inject_results() {
        let constellation = constellation_with(2);
        // A single (possibly malicious) peer returns junk — nothing is trusted.
        let hits = vec![peer(
            0.9,
            &["https://evil.example/1", "https://evil.example/2"],
        )];
        assert!(constellation.consensus(&hits, 10).is_empty());
    }

    #[test]
    fn search_consensus_enforces_source_floor_even_when_cfg_relaxes() {
        // Search results are inherently `Source::SearchEngine`, which has a
        // floor of 2 regardless of `cfg.min_agreement`. A user that drops
        // cfg to 1 (presumably to favor availability for some other
        // consult path) is intentionally NOT permitted to accept lone-peer
        // search results — there's no consumer-side verification to fall
        // back to for search hits, so a single (potentially malicious)
        // peer could otherwise inject results.
        let constellation = constellation_with(1);
        let hits = vec![peer(0.5, &["https://solo.example"])];
        assert!(constellation.consensus(&hits, 10).is_empty());

        // With two agreeing peers the result clears the floor.
        let hits = vec![
            peer(0.5, &["https://both.example"]),
            peer(0.4, &["https://both.example"]),
        ];
        let out = constellation.consensus(&hits, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://both.example");
    }

    #[test]
    fn add_peer_dedupes_and_caps() {
        let constellation = constellation_with(2);
        constellation.add_peer("http://a.example:8000");
        constellation.add_peer("http://a.example:8000/"); // same after normalization
        assert_eq!(constellation.peer_count(), 1);
        for i in 0..(MAX_GOSSIP_PEERS + 20) {
            constellation.add_peer(&format!("http://peer{i}.example:8000"));
        }
        assert!(constellation.peer_count() <= MAX_GOSSIP_PEERS);
    }

    #[test]
    fn reputation_persistence_round_trip() {
        let path =
            std::env::temp_dir().join(format!("lode-constellation-{}.json", std::process::id()));
        let path_str = path.to_string_lossy().to_string();
        let cfg = NetworkConfig {
            enabled: true,
            peers: vec!["http://a.example:8000".into()],
            state_file: path_str.clone(),
            ..NetworkConfig::default()
        };
        let constellation = Constellation::new(&cfg, Arc::new(TtlCache::new(60, 64)), None, None);
        constellation.persist_reputations();
        let loaded = load_reputations(&path_str);
        assert_eq!(loaded.get("http://a.example:8000").copied(), Some(0.5));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn answer_query_serves_local_and_guards_loops() {
        let constellation = constellation_with(2);
        let key = "abc123";
        let hits = vec![hit("https://x.example")];
        constellation
            .cache
            .put(key.to_string(), serde_json::to_string(&hits).unwrap());

        // ttl 0, not yet visited → served from our cache.
        assert_eq!(constellation.answer_query(key, 0, &[]).await.len(), 1);

        // Our own node id already in `seen` → loop guard returns nothing, even
        // though the entry is cached.
        let me = constellation.node_id().to_string();
        assert!(constellation.answer_query(key, 1, &[me]).await.is_empty());
    }

    #[test]
    fn constellation_id_configured_and_converges_to_larger_mesh() {
        let cfg = NetworkConfig {
            enabled: true,
            id: "mmm-mid".to_string(),
            ..NetworkConfig::default()
        };
        let c = Constellation::new(&cfg, Arc::new(TtlCache::new(60, 64)), None, None);
        assert_eq!(c.constellation_id(), "mmm-mid");
        // Our peer count is 0 (no static peers in default config).
        //
        // Equal mesh size (both 0): alphabetical tiebreaker. "zzz-high" loses,
        // "aaa-low" wins.
        c.maybe_adopt_id("zzz-high", 0);
        assert_eq!(c.constellation_id(), "mmm-mid");
        c.maybe_adopt_id("aaa-low", 0);
        assert_eq!(c.constellation_id(), "aaa-low");
        // Empty peer id never changes ours.
        c.maybe_adopt_id("", 0);
        assert_eq!(c.constellation_id(), "aaa-low");
    }

    #[test]
    fn larger_mesh_wins_over_alphabetically_smaller_id() {
        // A larger peer mesh overrides our id even if the peer's id is
        // alphabetically *greater* than ours — the mesh-size signal is
        // primary, alphabetical id is only the tiebreaker.
        let cfg = NetworkConfig {
            enabled: true,
            id: "aaa-low".to_string(),
            ..NetworkConfig::default()
        };
        let c = Constellation::new(&cfg, Arc::new(TtlCache::new(60, 64)), None, None);
        // Peer's mesh has 5 members; ours has 0. Peer wins despite
        // "zzz-high" > "aaa-low" alphabetically.
        c.maybe_adopt_id("zzz-high", 5);
        assert_eq!(c.constellation_id(), "zzz-high");
    }

    #[test]
    fn smaller_mesh_does_not_override_larger() {
        // Symmetric case: a peer in a smaller mesh can't drag us back even
        // if its id is alphabetically smaller. The big-mesh node already
        // adopted, and a stray small-mesh node poking in shouldn't undo it.
        let cfg = NetworkConfig {
            enabled: true,
            // Pretend we already merged to a bigger mesh's id.
            id: "zzz-big-mesh".to_string(),
            // Static peers seed our peer table so peer_count() > 0.
            peers: vec![
                "http://a.example:8000".into(),
                "http://b.example:8000".into(),
            ],
            ..NetworkConfig::default()
        };
        let c = Constellation::new(&cfg, Arc::new(TtlCache::new(60, 64)), None, None);
        // Our mesh has 2 peers. A lone peer offers an alphabetically smaller
        // id but its mesh is smaller (just itself, peer_count = 0). We
        // keep ours.
        c.maybe_adopt_id("aaa-tiny-mesh", 0);
        assert_eq!(c.constellation_id(), "zzz-big-mesh");
    }

    #[test]
    fn relay_guard_fires_once_per_window() {
        let constellation = constellation_with(2);
        // First relay of a key is allowed; an immediate duplicate is suppressed.
        assert!(constellation.should_relay("k1"));
        assert!(!constellation.should_relay("k1"));
        // A different key is independent.
        assert!(constellation.should_relay("k2"));
        assert!(!constellation.should_relay("k2"));
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
        let constellation = constellation_with(2);
        constellation.record_fetched("k", 1000);
        constellation.record_served("k", 1000);
        constellation.record_served("k", 1000);
        let s = constellation.seed_for("k").unwrap();
        assert_eq!((s.served, s.fetched), (2, 1));
        assert_eq!(s.ratio(), Some(2.0)); // gave back 2× what we took
    }
}
