//! WebSocket dashboard feed.
//!
//! A read-only push channel for the Nuxt frontend at `/ws/status`. The server
//! sends a snapshot of three subsystems on connect, then refreshes every
//! [`PUSH_INTERVAL`] seconds for as long as the socket is open:
//!
//! - **Server status** — uptime, build, tool/provider counts, basic config.
//! - **Memory stats** — counts from the SQLite memory store (memos, solutions,
//!   conversations, …). Cheap, indexed `COUNT(*)` queries — no row data.
//! - **Constellation state** — node id, constellation id, peer table summary,
//!   delegation knobs, seed-accounting totals.
//!
//! All messages are JSON-encoded with a tagged enum envelope (`type` +
//! `data`) so the frontend can pattern-match on the type and add new
//! variants without breaking older clients. No client-to-server commands
//! yet — this is a one-way feed for v1.
//!
//! ## Auth
//!
//! Same `[network].token` gate as the constellation endpoints. The frontend
//! passes the token via `?token=…` query string (so the browser's
//! `WebSocket` constructor — which can't set custom headers — still
//! authenticates). When no token is configured the endpoint is open.
//!
//! ## Privacy
//!
//! Snapshots contain **no secrets** and **no user content** — `<set>` /
//! `<unset>` redaction for keys (same convention as `features`), counts but
//! never row bodies for memory, peer URLs but never the cluster token.
//! Rule 11 applies here exactly as it does to the `features` tool.

use std::time::Duration;

use include_dir::{include_dir, Dir};
use serde::Serialize;

/// The Nuxt SPA's static-build output, embedded at compile time via
/// `build.rs`. When npm wasn't on PATH at compile time (or
/// `LODESTONE_SKIP_FRONTEND=1`), this directory is empty and the
/// `/dashboard` route returns a friendly "not built" page.
pub static DASHBOARD: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/frontend/.output/public");

/// How often the server pushes a fresh snapshot once a client is connected.
/// Short enough that the dashboard feels live; long enough that it doesn't
/// thrash a busy server.
pub const PUSH_INTERVAL: Duration = Duration::from_secs(5);

/// The tagged-enum message envelope. JSON shape:
/// `{"type":"server_status","data":{…}}` so the frontend can pattern-match
/// on `type` and treat each variant's `data` as a typed struct.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum WsMessage {
    /// A full snapshot bundle — all three subsystems at once. Sent on
    /// connect and on every push tick. Future variants might be incremental
    /// (e.g. `peer_changed`, `memo_added`) but the snapshot is the load-
    /// bearing message for v1.
    Snapshot(Snapshot),
}

/// The four-subsystem snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub server: ServerStatus,
    pub memory: MemoryStats,
    pub constellation: ConstellationState,
    pub browser: BrowserState,
}

/// Browser session manager snapshot — open sessions + named personas +
/// runtime knobs. Empty `sessions` + `personas` is normal (no model has
/// opened anything yet). The dashboard renders this on the `/browser`
/// page.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BrowserState {
    /// Live page URL + title + age + idle for every open session.
    pub sessions: Vec<crate::skills::browser_session::SessionSummary>,
    /// Named long-lived personas with their current state (`healthy` /
    /// `suspect` / `blocked`), last warning, and underlying session.
    /// Operator confirms reset via `POST /api/browser/personas/{name}/reset`.
    pub personas: Vec<crate::skills::browser_session::PersonaSummary>,
    /// Runtime-tunable: close a session idle this long. Mirrors
    /// `BrowserSessionConfig.idle_timeout_secs`.
    pub idle_timeout_secs: u64,
    /// Runtime-tunable: cap on concurrently open sessions.
    pub max_concurrent: usize,
}

/// Server-level information: build, uptime, what's active.
#[derive(Debug, Clone, Serialize)]
pub struct ServerStatus {
    /// `CARGO_PKG_VERSION` baked in at compile time.
    pub version: &'static str,
    /// `lodestone-mcp` for the main app.
    pub name: &'static str,
    /// Seconds since the server started. Frontend can format this for
    /// display (e.g. "3h 14m") without needing the server's local clock.
    pub uptime_secs: u64,
    /// Total tools the router currently exposes (post-gating).
    pub tools_active: usize,
    /// Tools gated off by config (e.g. `[memory].enabled = false` hides
    /// the memory tools). Useful in the UI as a "you have N tools hidden;
    /// here's which families" panel.
    pub tools_disabled: usize,
    /// The actual names of every active tool. The dashboard renders a
    /// drillable list grouped by family prefix so an operator can see
    /// *which* tools are reachable without calling `tools/list` over
    /// MCP. Ordered alphabetically.
    pub tools_active_names: Vec<String>,
    /// Names of tools gated off by the resolved config. Same shape as
    /// `tools_active_names`. Together the two lists cover every tool
    /// the build knows about.
    pub tools_disabled_names: Vec<String>,
    /// Tools the dashboard's settings drawer has flipped off at runtime.
    /// Disjoint from `tools_disabled_names` (those are config-disabled,
    /// these are session-disabled). Sorted.
    pub tools_runtime_disabled_names: Vec<String>,
    /// Active search providers — one entry per `(kind, id)`.
    pub providers: Vec<ProviderEntry>,
    /// Bind address the MCP listener accepted on. Read-only.
    pub bind: String,
    /// Bind address the constellation listener accepted on, or empty
    /// when the constellation shares the MCP port.
    pub constellation_bind: String,
    /// Per-secret presence flags — boolean only, never the value.
    /// `true` = configured (env or config). Golden rule 11: a secret's
    /// presence can be surfaced; the bytes never can.
    pub secrets: SecretPresence,
    /// Active tracing filter directive (e.g. `lodestone_mcp=info,rmcp=warn`).
    /// Mutated at runtime via `POST /api/settings/server { log_level }`.
    pub log_level: String,
}

/// One bit per secret the server might be configured with. The
/// dashboard renders these as `<set>` / `<unset>` badges so operators
/// can audit configuration without ever seeing the value.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SecretPresence {
    pub auth_token: bool,
    pub network_token: bool,
    pub github_token: bool,
    pub nasa_key: bool,
    pub eia_key: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderEntry {
    pub kind: String,
    pub id: String,
}

/// Memory-store counts. None of these carry row bodies — just `COUNT(*)`s
/// from the indexed tables. If the memory family is disabled (`[memory]
/// .enabled = false`) all fields are zero.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MemoryStats {
    pub enabled: bool,
    pub memos: u64,
    pub solutions: u64,
    pub solution_revisions: u64,
    pub solution_tags: u64,
    pub solution_links: u64,
    pub solution_phrasings: u64,
    pub conversations: u64,
    pub conversation_turns: u64,
    pub synonyms: u64,
    /// Resolved memory store directory. Read-only; the path is
    /// established at startup. Empty when memory is disabled.
    pub db_path: String,
    /// Resolved embedding model id used for semantic memory search.
    /// Read-only; switching models requires a restart so the index
    /// vectors stay consistent. Empty when memory is disabled or
    /// embeddings aren't configured.
    pub embedding_model: String,
    /// Auto-recall preamble currently armed. Runtime-tunable.
    pub auto_recall: bool,
    /// Conversation-turn recording currently armed. Runtime-tunable.
    pub record_conversations: bool,
}

/// Constellation snapshot — peer table + identity + delegation knobs.
/// When `[network].enabled = false`, `enabled` is `false` and the rest is
/// zero / empty so the frontend can render "constellation disabled" rather
/// than crash on missing fields.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ConstellationState {
    pub enabled: bool,
    /// This node's stable id.
    pub node_id: String,
    /// The shared constellation id (may differ from configured id after a
    /// merge — see `maybe_adopt_id`).
    pub constellation_id: String,
    /// Number of peers in the table (reachable + still-being-tried).
    pub peer_count: usize,
    /// Per-peer summary suitable for a UI list — never carries the
    /// cluster token.
    pub peers: Vec<PeerEntry>,
    /// True if this node advertised `delegation_enabled = true`.
    pub delegation_enabled: bool,
    pub delegation_max_jobs_per_peer_per_hour: u32,
    pub delegation_max_bytes_per_job: u64,
    pub delegation_total_bytes_per_hour: u64,
    /// Aggregate seed-accounting bytes (BitTorrent-style): how much this
    /// node has served vs. fetched across all blobs.
    pub total_served_bytes: u64,
    pub total_fetched_bytes: u64,
    /// URLs that resolve to this very node. Lets the dashboard recognize
    /// "peer X says it knows us" without false positives when a peer
    /// references us by a LAN address rather than localhost.
    pub local_urls: Vec<String>,
    /// `max_peers` cap currently in effect. Mirrors the runtime
    /// override, not the config value, so the dashboard settings drawer
    /// sees its own edits take effect.
    pub max_peers: usize,
    /// Min-agreement consensus floor currently in effect.
    pub min_agreement: usize,
    /// Config-file values for the knobs that aren't runtime-tunable
    /// (changing them requires a restart). The settings drawer shows
    /// these read-only with a "restart required" badge.
    pub mdns_configured: bool,
    pub sync_secs_configured: u64,
    pub request_timeout_ms_configured: u64,
    /// What this node advertises to the rest of the constellation —
    /// the per-feature `query` / `retrieval` / `blob` / `browser`
    /// opt-in set. Mirrors `[network].capabilities`.
    pub local_capabilities: crate::config::Capabilities,
}

#[derive(Debug, Clone, Serialize)]
pub struct PeerEntry {
    pub url: String,
    pub node_id: Option<String>,
    pub reputation: f64,
    /// Whether we currently hold a valid Bloom filter for this peer (i.e.
    /// have successfully fetched its digest recently).
    pub reachable: bool,
    /// Did this peer advertise willingness to serve delegated retrieves?
    pub delegation_enabled: bool,
    /// URLs this peer advertised as its own known neighbors on its most
    /// recent digest. Drives the swarm view's peer-to-peer edges. Empty
    /// until we've fetched the peer's digest at least once.
    pub known_peers: Vec<String>,
    /// Per-feature opt-in set this peer advertised. `None` until we've
    /// successfully fetched its digest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<crate::config::Capabilities>,
}
