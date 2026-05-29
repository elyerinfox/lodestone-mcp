//! The galaxy **client** — the participating side, embedded in the main
//! `lodestone-mcp` app. When `[galaxy].servers` is set (and the constellation is
//! enabled), this registers the constellation's public ingress endpoints with each
//! galaxy broker and pulls the directory, adding *other* constellations' endpoints
//! as peers so the existing consult path reaches them directly. The broker itself
//! is a separate program (`lodestone-galaxy`); this only talks to it.

use std::sync::Arc;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;

use crate::constellation::Constellation;

/// A constellation's directory entry (deserialized from a broker's response). Mirrors
/// the broker's wire type; kept here so the main app doesn't compile the broker.
#[derive(Debug, Deserialize)]
struct DirectoryEntry {
    id: String,
    #[serde(default)]
    endpoints: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DirectoryResp {
    #[serde(default)]
    constellations: Vec<DirectoryEntry>,
}

/// Settings for the participating side (this constellation joining brokers).
pub struct GalaxyClient {
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
    /// Spawn the background loop. A node first **joins its own constellation** (a
    /// warm-up that returns early once a local peer appears), then every
    /// `heartbeat_secs` registers with each broker and pulls its directory, adding
    /// other constellations' endpoints as peers. Best-effort.
    pub fn start(self, constellation: Arc<Constellation>) {
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

    /// Wait until the local constellation has formed (≥1 known peer) or the warm-up
    /// elapses — whichever first — so a lone node still eventually reaches out.
    async fn await_local_constellation(&self, constellation: &Constellation) {
        let mut waited = 0u64;
        let cap = self.join_warmup_secs.max(1);
        loop {
            if constellation.known_peer_count() > 0 {
                tracing::info!("galaxy: local constellation has peers; reaching out to brokers");
                return;
            }
            if waited >= cap {
                tracing::info!("galaxy: warm-up elapsed; reaching out to brokers");
                return;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
            waited += 2;
        }
    }

    /// One register + directory-pull against a single broker.
    async fn sync_once(&self, server: &str, constellation: &Constellation) {
        let base = server.trim_end_matches('/');
        // Register under the SHARED constellation id (so all member nodes appear as
        // one constellation), unless an explicit `[galaxy].id` overrides it. Read it
        // fresh each cycle since the id can converge as meshes merge.
        let id = if self.id.trim().is_empty() {
            constellation.constellation_id()
        } else {
            self.id.clone()
        };
        let reg = serde_json::json!({ "id": id, "endpoints": self.ingress });
        let mut post = self.http.post(format!("{base}/galaxy/register")).json(&reg);
        if !self.token.is_empty() {
            post = post.bearer_auth(&self.token);
        }
        if let Err(e) = post.send().await {
            tracing::debug!(server = base, error = %e, "galaxy register failed");
            return;
        }
        let mut get = self
            .http
            .get(format!("{base}/galaxy/directory?id={}", urlencode(&id)));
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
            if entry.id == id {
                continue;
            }
            for ep in &entry.endpoints {
                if self.ingress.iter().any(|m| m == ep) {
                    continue; // never peer with our own advertised ingress
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
    fn urlencode_escapes() {
        assert_eq!(urlencode("alpha-1"), "alpha-1");
        assert_eq!(urlencode("a b/c"), "a%20b%2Fc");
    }
}
