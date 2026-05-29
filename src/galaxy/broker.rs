//! The galaxy **broker** — a standalone rendezvous directory that links
//! constellations across networks. It is deliberately **not a proxy**: it never
//! relays digests, queries, or blobs. It only stores `{ constellation_id → public
//! endpoint(s) }` so constellations can discover each other and then talk
//! **directly**, peer-to-peer, over the normal `/constellation/*` endpoints.
//!
//! This file is self-contained (no `crate::` dependencies) so it compiles both into
//! the standalone `lodestone-galaxy` binary (`src/bin/lodestone-galaxy.rs`, via
//! `#[path]`) and as a module of any host that wants to embed it. The main
//! `lodestone-mcp` binary does **not** embed it — running a broker is a separate
//! program; the MCP server and its constellation are the main app.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Constant-time byte comparison (avoids leaking the token via timing).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// One constellation's registration in the broker directory.
#[derive(Clone)]
struct Reg {
    endpoints: Vec<String>,
    last_seen: u64,
}

/// A constellation's public ingress entry, as returned by the directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DirectoryEntry {
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
pub struct GalaxyBroker {
    dir: Mutex<HashMap<String, Reg>>,
    token: String,
    ttl: Duration,
}

impl GalaxyBroker {
    pub fn new(token: &str, ttl_secs: u64) -> Arc<Self> {
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
pub fn galaxy_routes(broker: Arc<GalaxyBroker>) -> axum::Router {
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
        let seen = b.list("alpha");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].id, "beta");
        let seen = b.list("beta");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].endpoints.len(), 2);
    }

    #[test]
    fn stale_entries_are_evicted() {
        let b = GalaxyBroker::new("", 60);
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
        let open = GalaxyBroker::new("", 60);
        assert!(open.token_ok(None));
    }

    #[test]
    fn empty_id_register_ignored() {
        let b = GalaxyBroker::new("", 60);
        b.register("  ", vec!["http://x:8001".into()]);
        assert!(b.list("nobody").is_empty());
    }

    #[test]
    fn ct_eq_basic() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
