//! The galaxy — a rendezvous **broker** that links *constellations* across
//! networks. It is deliberately **not a proxy**: it never relays digests, queries,
//! or blobs. It only keeps a directory of `{ constellation_id → public endpoint(s) }`
//! so that constellations behind different networks can discover each other and then
//! talk **directly**, peer-to-peer, over the existing `/constellation/*` endpoints.
//!
//! Topology: at least one host must be publicly reachable. Typically that's the
//! galaxy broker itself (a tiny public service), and each constellation advertises
//! one or more **ingress** URLs (publicly-reachable member nodes). A constellation
//! may advertise *several* ingress endpoints; peers add them all, spreading inbound
//! load across them. Egress is naturally distributed too — every member node runs
//! its own galaxy client and registers independently.
//!
//! Two roles, both opt-in via `[galaxy]`:
//!   * **serve** — run the broker (the public directory).
//!   * **participate** — register this constellation with one or more brokers and
//!     pull the directory, adding other constellations' ingress endpoints as peers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::constellation::Constellation;
use crate::util::ct_eq;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One constellation's registration in the broker directory.
#[derive(Clone)]
struct Reg {
    endpoints: Vec<String>,
    last_seen: u64,
}

/// A constellation's public ingress entry, as returned by the directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct DirectoryEntry {
    pub id: String,
    pub endpoints: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RegisterReq {
    id: String,
    #[serde(default)]
    endpoints: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct DirectoryResp {
    constellations: Vec<DirectoryEntry>,
}

/// The broker's in-memory directory. Entries expire after `ttl` without a refresh.
pub(crate) struct GalaxyBroker {
    dir: Mutex<HashMap<String, Reg>>,
    token: String,
    ttl: Duration,
}

impl GalaxyBroker {
    pub(crate) fn new(token: &str, ttl_secs: u64) -> Arc<Self> {
        Arc::new(Self {
            dir: Mutex::new(HashMap::new()),
            token: token.to_string(),
            ttl: Duration::from_secs(ttl_secs.max(1)),
        })
    }

    /// Constant-time bearer check; always ok when no token is configured.
    fn token_ok(&self, presented: Option<&str>) -> bool {
        if self.token.is_empty() {
            return true;
        }
        presented.is_some_and(|t| ct_eq(t.as_bytes(), self.token.as_bytes()))
    }

    /// Upsert a constellation's endpoints and stamp it fresh.
    fn register(&self, id: &str, endpoints: Vec<String>) {
        if id.trim().is_empty() {
            return;
        }
        let mut dir = self.dir.lock().unwrap();
        dir.insert(
            id.trim().to_string(),
            Reg {
                endpoints,
                last_seen: now_secs(),
            },
        );
    }

    /// The current directory minus `exclude` and any entries past their TTL (which
    /// are also evicted). So a caller never gets itself or stale constellations back.
    fn list(&self, exclude: &str) -> Vec<DirectoryEntry> {
        let now = now_secs();
        let ttl = self.ttl.as_secs();
        let mut dir = self.dir.lock().unwrap();
        dir.retain(|_, r| now.saturating_sub(r.last_seen) <= ttl);
        dir.iter()
            .filter(|(id, _)| id.as_str() != exclude)
            .map(|(id, r)| DirectoryEntry {
                id: id.clone(),
                endpoints: r.endpoints.clone(),
            })
            .collect()
    }
}

/// Bearer token from an `Authorization: Bearer <t>` header, if present.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

#[derive(Debug, Deserialize)]
struct DirQuery {
    /// The caller's own id, excluded from the result.
    #[serde(default)]
    id: String,
}

/// Axum router for the broker: `POST /galaxy/register` (also serves as a heartbeat)
/// and `GET /galaxy/directory?id=<self>`. No query/digest/blob routes exist here —
/// the broker never proxies constellation traffic.
pub(crate) fn galaxy_routes(broker: Arc<GalaxyBroker>) -> axum::Router {
    async fn register(
        State(b): State<Arc<GalaxyBroker>>,
        headers: HeaderMap,
        axum::Json(req): axum::Json<RegisterReq>,
    ) -> axum::response::Response {
        if !b.token_ok(bearer(&headers)) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        b.register(&req.id, req.endpoints);
        axum::Json(serde_json::json!({ "ok": true })).into_response()
    }

    async fn directory(
        State(b): State<Arc<GalaxyBroker>>,
        headers: HeaderMap,
        Query(q): Query<DirQuery>,
    ) -> axum::response::Response {
        if !b.token_ok(bearer(&headers)) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        axum::Json(DirectoryResp {
            constellations: b.list(&q.id),
        })
        .into_response()
    }

    axum::Router::new()
        .route("/galaxy/register", post(register))
        .route("/galaxy/heartbeat", post(register))
        .route("/galaxy/directory", get(directory))
        .with_state(broker)
}

/// Settings for the participating side (this constellation joining brokers).
pub(crate) struct GalaxyClient {
    pub http: Client,
    pub servers: Vec<String>,
    pub id: String,
    pub ingress: Vec<String>,
    pub token: String,
    pub heartbeat_secs: u64,
    /// How long to let *local* constellation discovery (static peers, mDNS, gossip)
    /// settle before contacting any galaxy broker — so a node first learns what
    /// constellation it belongs to, then looks outward for others.
    pub join_warmup_secs: u64,
}

impl GalaxyClient {
    /// Spawn the background loop. A node first **joins its own constellation**: we
    /// wait out a warm-up (capped by whether local peers have appeared) so local
    /// discovery settles before we register with, or query, any galaxy broker for
    /// *other* constellations. Then, every `heartbeat_secs`, register with each broker
    /// and pull its directory, adding other constellations' endpoints as peers so the
    /// existing consult path reaches them directly. Best-effort.
    pub(crate) fn start(self, constellation: Arc<Constellation>) {
        tokio::spawn(async move {
            self.await_local_constellation(&constellation).await;
            let period = Duration::from_secs(self.heartbeat_secs.max(5));
            loop {
                for server in &self.servers {
                    self.sync_once(server, &constellation).await;
                }
                tokio::time::sleep(period).await;
            }
        });
    }

    /// Block until the local constellation has had a chance to form: return as soon
    /// as at least one local peer is known, or when `join_warmup_secs` elapses
    /// (whichever first), so a lone node still eventually reaches out.
    async fn await_local_constellation(&self, constellation: &Constellation) {
        let deadline = now_secs() + self.join_warmup_secs.max(1);
        loop {
            if constellation.known_peer_count() > 0 {
                tracing::info!("galaxy: local constellation has peers; reaching out to brokers");
                return;
            }
            if now_secs() >= deadline {
                tracing::info!("galaxy: warm-up elapsed; reaching out to brokers");
                return;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    /// One register + directory-pull against a single broker.
    async fn sync_once(&self, server: &str, constellation: &Constellation) {
        let base = server.trim_end_matches('/');
        let reg = serde_json::json!({ "id": self.id, "endpoints": self.ingress });
        let mut post = self.http.post(format!("{base}/galaxy/register")).json(&reg);
        if !self.token.is_empty() {
            post = post.bearer_auth(&self.token);
        }
        if let Err(e) = post.send().await {
            tracing::debug!(server = base, error = %e, "galaxy register failed");
            return;
        }
        let mut get = self.http.get(format!(
            "{base}/galaxy/directory?id={}",
            urlencode(&self.id)
        ));
        if !self.token.is_empty() {
            get = get.bearer_auth(&self.token);
        }
        let resp = match get
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(server = base, error = %e, "galaxy directory fetch failed");
                return;
            }
        };
        let dir: DirectoryResp = match resp.json::<DirectoryResp>().await {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!(server = base, error = %e, "galaxy directory parse failed");
                return;
            }
        };
        let mut added = 0usize;
        for entry in &dir.constellations {
            if entry.id == self.id {
                continue;
            }
            for ep in &entry.endpoints {
                // Don't peer with our own advertised ingress endpoints.
                if self.ingress.iter().any(|m| m == ep) {
                    continue;
                }
                constellation.add_peer(ep);
                added += 1;
            }
        }
        if added > 0 {
            tracing::info!(
                server = base,
                constellations = dir.constellations.len(),
                endpoints = added,
                "galaxy: linked constellations as peers"
            );
        }
    }
}

/// Minimal percent-encoding for the `id` query parameter.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_directory_excludes_self() {
        let b = GalaxyBroker::new("", 60);
        b.register(
            "alpha",
            vec!["http://a1:8001".into(), "http://a2:8001".into()],
        );
        b.register("beta", vec!["http://b1:8001".into()]);
        // alpha asks → sees only beta.
        let seen = b.list("alpha");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].id, "beta");
        // beta asks → sees alpha with BOTH ingress endpoints (distributed ingress).
        let seen = b.list("beta");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].endpoints.len(), 2);
    }

    #[test]
    fn stale_entries_are_evicted() {
        let b = GalaxyBroker::new("", 60);
        // Insert a stale entry by hand.
        b.dir.lock().unwrap().insert(
            "ghost".into(),
            Reg {
                endpoints: vec!["http://x:8001".into()],
                last_seen: now_secs().saturating_sub(120),
            },
        );
        b.register("live", vec!["http://y:8001".into()]);
        let seen = b.list("nobody");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].id, "live");
    }

    #[test]
    fn token_gates_access() {
        let b = GalaxyBroker::new("secret", 60);
        assert!(b.token_ok(Some("secret")));
        assert!(!b.token_ok(Some("wrong")));
        assert!(!b.token_ok(None));
        // No token configured → open.
        let open = GalaxyBroker::new("", 60);
        assert!(open.token_ok(None));
    }

    #[test]
    fn empty_id_register_ignored() {
        let b = GalaxyBroker::new("", 60);
        b.register("  ", vec!["http://x:8001".into()]);
        assert!(b.list("nobody").is_empty());
    }
}
