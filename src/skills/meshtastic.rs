//! Meshtastic skill — read mesh messages, node info, and send text messages.
//!
//! **Transport (v1): MQTT only.** Meshtastic nodes that have firmware MQTT
//! JSON output enabled publish onto topics shaped like
//! `<root>/<region>/2/json/<channel>/<node>` carrying a JSON envelope with
//! `from`, `to`, `channel`, `payload`, `rssi`, `snr`, `timestamp`, `type`.
//! This module is a *decoder layer*: it leans on the shared
//! [`crate::skills::mqtt::MqttClient`] for the actual transport (one
//! connection, one event loop, one ring buffer) and translates the
//! Meshtastic-specific topic / payload shape into mesh semantics.
//!
//! Serial / TCP / BLE transports + protobuf decode are deferred to a
//! follow-up — they require the upstream Meshtastic `.proto` files and a
//! `prost`-based build step. The JSON-over-MQTT path covers the most
//! common bridge configuration today (a node forwarding to
//! `mqtt.meshtastic.org` or a self-hosted broker).
//!
//! Off by default. Requires `[mqtt].enabled` plus a reachable broker — the
//! family probe surfaces an actionable error when MQTT isn't wired up.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::config;
use crate::skills::mqtt::{MqttClient, MqttMessage};
use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

/// Topic-filter wildcard that matches every Meshtastic JSON publish under
/// the configured root. Used both for the auto-subscribe and for the
/// `mqtt.recent` filter when reading.
pub(crate) fn topic_filter(cfg: &config::Meshtastic) -> String {
    format!("{}/+/2/json/#", cfg.mqtt_topic_root.trim_end_matches('/'))
}

/// Format a node id (`!11223344` or numeric) into a stable display string.
/// Meshtastic uses both representations in different fields; the `from` /
/// `to` numerics are 32-bit; `sender` is `!<hex>`. We normalize to lowercase
/// hex with a `!` prefix in summaries.
fn fmt_node(value: &Value) -> String {
    if let Some(s) = value.as_str() {
        if let Some(stripped) = s.strip_prefix('!') {
            return format!("!{}", stripped.to_ascii_lowercase());
        }
        return s.to_string();
    }
    if let Some(n) = value.as_u64() {
        return format!("!{:08x}", n);
    }
    if let Some(n) = value.as_i64() {
        return format!("!{:08x}", n as u32);
    }
    "?".into()
}

fn require_client(server: &crate::Lodestone) -> Result<&Arc<MqttClient>, McpError> {
    server.mqtt.as_ref().ok_or_else(|| {
        invalid(
            "Meshtastic uses the MQTT transport but [mqtt] isn't wired up — enable [mqtt] and \
             configure [mqtt].broker to point at a Meshtastic-bridged broker.",
        )
    })
}

fn cfg_for(server: &crate::Lodestone) -> &config::Meshtastic {
    &server.cfg.meshtastic
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MessagesArgs {
    /// Optional channel name filter (e.g. `LongFast`). Omit for every channel.
    #[serde(default)]
    channel: Option<String>,
    /// Optional sender filter (`!11223344` form). Matches the `sender` field
    /// or the `from` numeric.
    #[serde(default)]
    from: Option<String>,
    /// Cap on rows returned (default 20, max 200; newest first).
    #[serde(default)]
    limit: Option<usize>,
}

pub struct MeshtasticMessages;
impl Skill for MeshtasticMessages {
    fn name(&self) -> &'static str {
        "meshtastic_messages"
    }
    fn description(&self) -> &'static str {
        "Recent text messages observed on the Meshtastic mesh, decoded from the JSON \
        MQTT topic format (newest first). Optional `channel` and `from` filters. Requires \
        a Meshtastic node bridging the mesh to the MQTT broker configured in [mqtt]."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MessagesArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<MessagesArgs>()?;
            let client = require_client(server)?;
            let cfg = cfg_for(server);
            let filter = topic_filter(cfg);
            let limit = args.limit.unwrap_or(20).clamp(1, 200);
            // Pull more than `limit` from the MQTT buffer because some
            // matched topics will be telemetry / nodeinfo (not text) and
            // get filtered out below.
            let raw = client.recent(Some(&filter), limit * 4).await;
            let mut rows: Vec<String> = Vec::new();
            for msg in raw {
                let Some(entry) = decode_text(&msg, &args.channel, args.from.as_deref()) else {
                    continue;
                };
                rows.push(entry);
                if rows.len() >= limit {
                    break;
                }
            }
            if rows.is_empty() {
                return Ok(text_result(
                    "No matching mesh text messages buffered (yet). Check that \
                     [meshtastic].auto_subscribe is true (or run `mqtt_subscribe` against \
                     the meshtastic topic filter), and that a node is forwarding to the \
                     broker.",
                ));
            }
            Ok(text_result(format!(
                "{} message(s):\n{}",
                rows.len(),
                rows.join("\n")
            )))
        })
    }
}

/// Decode one MQTT publish into a one-line text-message summary, or `None`
/// if it isn't a text message (telemetry / nodeinfo / etc.) or doesn't
/// match the caller's filters.
fn decode_text(msg: &MqttMessage, channel: &Option<String>, from: Option<&str>) -> Option<String> {
    let v: Value = serde_json::from_slice(&msg.payload).ok()?;
    // Topic shape: msh/<region>/2/json/<channel>/<node>
    let topic_parts: Vec<&str> = msg.topic.split('/').collect();
    let topic_channel = topic_parts.get(4).copied();
    if let Some(want) = channel.as_deref() {
        if topic_channel != Some(want) {
            return None;
        }
    }
    let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
    if kind != "text" && kind != "TEXT_MESSAGE_APP" {
        return None;
    }
    let sender = v
        .get("sender")
        .map(fmt_node)
        .or_else(|| v.get("from").map(fmt_node))
        .unwrap_or_else(|| "?".into());
    if let Some(want) = from {
        if !sender.eq_ignore_ascii_case(want) {
            return None;
        }
    }
    let to = v.get("to").map(fmt_node).unwrap_or_else(|| "?".into());
    let body = v
        .get("payload")
        .and_then(|p| p.get("text"))
        .and_then(Value::as_str)
        .or_else(|| v.get("payload").and_then(Value::as_str))
        .unwrap_or("");
    let rssi = v.get("rssi").and_then(Value::as_i64);
    let snr = v.get("snr").and_then(Value::as_f64);
    let ts = v
        .get("timestamp")
        .and_then(Value::as_i64)
        .unwrap_or(msg.received_ms / 1000);
    let mut line = format!(
        "[{ts}] {sender} → {to} on {ch}: {body}",
        ch = topic_channel.unwrap_or("?"),
    );
    if let (Some(r), Some(s)) = (rssi, snr) {
        line.push_str(&format!("  (rssi {r}, snr {s:.1})"));
    }
    Some(line)
}

pub struct MeshtasticNodes;
impl Skill for MeshtasticNodes {
    fn name(&self) -> &'static str {
        "meshtastic_nodes"
    }
    fn description(&self) -> &'static str {
        "List Meshtastic nodes recently heard on the mesh, derived from `nodeinfo` and \
        `telemetry` JSON messages buffered from MQTT. Includes node id, long/short name \
        when known, last RSSI / SNR, last-seen timestamp. Coverage is whatever has crossed \
        the buffer since startup."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let client = require_client(ctx.server)?;
            let cfg = cfg_for(ctx.server);
            let filter = topic_filter(cfg);
            // Walk the whole buffer (capped). Newest msg per node wins.
            let raw = client
                .recent(Some(&filter), client.buffer_capacity.max(64))
                .await;
            let mut nodes: HashMap<String, NodeRow> = HashMap::new();
            for msg in raw.iter().rev() {
                // Oldest-first so newer overwrites older fields.
                let Ok(v) = serde_json::from_slice::<Value>(&msg.payload) else {
                    continue;
                };
                let sender = v
                    .get("sender")
                    .map(fmt_node)
                    .or_else(|| v.get("from").map(fmt_node));
                let Some(id) = sender else { continue };
                let row = nodes.entry(id.clone()).or_insert_with(|| NodeRow {
                    id: id.clone(),
                    long_name: None,
                    short_name: None,
                    rssi: None,
                    snr: None,
                    last_seen_ms: msg.received_ms,
                });
                row.last_seen_ms = row.last_seen_ms.max(msg.received_ms);
                if let Some(ln) = v
                    .get("payload")
                    .and_then(|p| p.get("longname"))
                    .and_then(Value::as_str)
                    .or_else(|| v.get("longname").and_then(Value::as_str))
                {
                    row.long_name = Some(ln.to_string());
                }
                if let Some(sn) = v
                    .get("payload")
                    .and_then(|p| p.get("shortname"))
                    .and_then(Value::as_str)
                    .or_else(|| v.get("shortname").and_then(Value::as_str))
                {
                    row.short_name = Some(sn.to_string());
                }
                if let Some(r) = v.get("rssi").and_then(Value::as_i64) {
                    row.rssi = Some(r);
                }
                if let Some(s) = v.get("snr").and_then(Value::as_f64) {
                    row.snr = Some(s);
                }
            }
            if nodes.is_empty() {
                return Ok(text_result(
                    "No mesh nodes observed yet. Wait for `nodeinfo` / `text` traffic to \
                     buffer, or check that [meshtastic].auto_subscribe is true.",
                ));
            }
            let mut rows: Vec<NodeRow> = nodes.into_values().collect();
            rows.sort_by_key(|r| std::cmp::Reverse(r.last_seen_ms));
            let mut out = format!("{} node(s) heard:", rows.len());
            for r in rows {
                let name = match (&r.long_name, &r.short_name) {
                    (Some(l), Some(s)) => format!("{l} ({s})"),
                    (Some(l), None) => l.clone(),
                    (None, Some(s)) => s.clone(),
                    (None, None) => "<unknown>".into(),
                };
                let mut line = format!("\n  {} — {name}", r.id);
                if let Some(rssi) = r.rssi {
                    line.push_str(&format!("  rssi={rssi}"));
                }
                if let Some(snr) = r.snr {
                    line.push_str(&format!("  snr={snr:.1}"));
                }
                line.push_str(&format!("  last-seen={}ms-ago", now_ms() - r.last_seen_ms));
                out.push_str(&line);
            }
            Ok(text_result(out))
        })
    }
}

struct NodeRow {
    id: String,
    long_name: Option<String>,
    short_name: Option<String>,
    rssi: Option<i64>,
    snr: Option<f64>,
    last_seen_ms: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SendArgs {
    /// Text body to broadcast (UTF-8, ≤ ~200 chars to fit a single LoRa frame).
    text: String,
    /// Channel name. Omit for `[meshtastic].default_channel`.
    #[serde(default)]
    channel: Option<String>,
    /// Region segment for the topic. Omit for `[meshtastic].default_region`.
    #[serde(default)]
    region: Option<String>,
    /// Destination node — `!11223344` form or `^all` for broadcast (default).
    #[serde(default)]
    to: Option<String>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, stop asking for `meshtastic_send` for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

pub struct MeshtasticSend;
impl Skill for MeshtasticSend {
    fn name(&self) -> &'static str {
        "meshtastic_send"
    }
    fn description(&self) -> &'static str {
        "Publish a text message onto the Meshtastic mesh by formatting the firmware's \
        JSON envelope and PUBLISHing to `<root>/<region>/2/json/<channel>/<to>`. The \
        bridging node decodes + retransmits on LoRa. **Side-effecting** — broadcasts \
        on a physical LoRa mesh. First call returns a confirmation token and does \
        nothing; call again with `confirm=<token>` to send (or `confirm + trust=true`). \
        `[meshtastic].allow_destructive=true` pre-authorizes. `to` defaults to `^all` \
        (broadcast); `channel` / `region` default to `[meshtastic]`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SendArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use crate::skills::guard::Decision;
            let (server, args) = ctx.parse::<SendArgs>()?;
            let client = require_client(server)?;
            let cfg = cfg_for(server);
            if args.text.trim().is_empty() {
                return Err(invalid("text is required"));
            }
            if args.text.len() > 220 {
                return Err(invalid(
                    "Meshtastic text payloads must fit a single LoRa frame (~200 bytes)",
                ));
            }
            let channel = args
                .channel
                .as_deref()
                .unwrap_or(cfg.default_channel.as_str());
            let region = args
                .region
                .as_deref()
                .unwrap_or(cfg.default_region.as_str());
            let to = args.to.as_deref().unwrap_or("^all");
            let topic = format!(
                "{root}/{region}/2/json/{channel}/{to}",
                root = cfg.mqtt_topic_root.trim_end_matches('/'),
            );
            let summary = format!(
                "broadcast {} byte(s) to mesh channel '{channel}' (to={to}) via {}",
                args.text.len(),
                client.broker()
            );
            if let Decision::Challenge(msg) = server.guard.check(
                "meshtastic_send",
                "meshtastic_send",
                cfg.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let envelope = serde_json::json!({
                "type": "sendtext",
                "channel": channel,
                "payload": args.text,
                "to": to,
            });
            let bytes = serde_json::to_vec(&envelope).map_err(|e| internal(anyhow!(e)))?;
            client
                .publish(&topic, client.default_qos, false, bytes)
                .await
                .map_err(internal)?;
            Ok(text_result(format!(
                "Sent {} byte(s) to {topic} via {}.",
                args.text.len(),
                client.broker()
            )))
        })
    }
}

pub struct MeshtasticStatus;
impl Skill for MeshtasticStatus {
    fn name(&self) -> &'static str {
        "meshtastic_status"
    }
    fn description(&self) -> &'static str {
        "Report Meshtastic skill state: transport, topic root, default channel / region, \
        whether the underlying MQTT client is wired up, and the count of mesh-matching \
        messages currently in the buffer."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let cfg = cfg_for(ctx.server);
            let mut out = format!(
                "Meshtastic\n  transport: {}\n  mqtt_topic_root: {}\n  default_region: {}\n  \
                 default_channel: {}\n  auto_subscribe: {}",
                cfg.transport,
                cfg.mqtt_topic_root,
                cfg.default_region,
                cfg.default_channel,
                cfg.auto_subscribe,
            );
            match ctx.server.mqtt.as_ref() {
                Some(client) => {
                    let filter = topic_filter(cfg);
                    let count = client
                        .recent(Some(&filter), client.buffer_capacity)
                        .await
                        .len();
                    out.push_str(&format!(
                        "\n  mqtt: wired ({})\n  mesh-matching buffered: {count}",
                        client.broker()
                    ));
                }
                None => {
                    out.push_str("\n  mqtt: not wired ([mqtt] disabled or broker unreachable)");
                }
            }
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListenArgs {
    /// Optional channel filter (e.g. `LongFast`). Omit for every channel.
    #[serde(default)]
    channel: Option<String>,
    /// Stop after collecting this many text messages. Default 25, max 500.
    #[serde(default)]
    max_messages: Option<usize>,
    /// Stop after this many seconds even if fewer arrived. Default 120, max 3600.
    #[serde(default)]
    timeout_secs: Option<u64>,
}

pub struct MeshtasticListen;
impl Skill for MeshtasticListen {
    fn name(&self) -> &'static str {
        "meshtastic_listen"
    }
    fn description(&self) -> &'static str {
        "Spawn an async task watching the Meshtastic mesh; each new text message \
        fires a `notifications/progress` (correlated by `_meta.progressToken`) and \
        the buffered list is fetchable via `tasks_result`. Lifecycle changes push \
        `notifications/tasks/status`. Stops at `max_messages`, `timeout_secs`, or \
        `tasks_cancel`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ListenArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let peer = ctx.peer.clone();
            let token = ctx.progress_token();
            let (server, args) = ctx.parse::<ListenArgs>()?;
            let client = require_client(server)?.clone();
            let cfg = cfg_for(server);
            let filter = topic_filter(cfg);
            let channel = args.channel.clone();
            let max_messages = args.max_messages.unwrap_or(25).clamp(1, 500);
            let timeout =
                std::time::Duration::from_secs(args.timeout_secs.unwrap_or(120).clamp(1, 3600));
            // Make sure we're subscribed (no-op if auto_subscribe already did).
            client
                .subscribe(&filter, client.default_qos)
                .await
                .map_err(crate::internal)?;
            let runtime = server.task_runtime.clone();
            let runtime_for_observers = runtime.clone();
            let label = match &channel {
                Some(c) => format!("meshtastic_listen channel={c}"),
                None => "meshtastic_listen any-channel".to_string(),
            };
            let task_id = runtime
                .spawn("meshtastic_listen", label, move |handle| async move {
                    listen_body(handle, client, filter, channel, max_messages, timeout).await
                })
                .await;
            if let (Some(peer), Some(token)) = (peer.clone(), token) {
                runtime_for_observers
                    .observe_progress(&task_id, peer, token)
                    .await;
            }
            if let Some(peer) = peer {
                runtime_for_observers.observe_status(&task_id, peer).await;
            }
            Ok(text_result(format!(
                "Listening on the mesh (task_id={task_id}). Per-message progress via \
                 `notifications/progress`, completion via `notifications/tasks/status`, \
                 buffered messages via `tasks_result {{\"task_id\":\"{task_id}\"}}`."
            )))
        })
    }
}

/// Watch for decoded mesh TEXT messages and emit one progress per match.
/// Same cursor-by-`received_ms` polling pattern as `mqtt_listen`; the
/// difference is the per-message filter routes through `decode_text` so
/// non-text Meshtastic traffic (telemetry, nodeinfo, position) doesn't
/// surface to the model.
async fn listen_body(
    handle: crate::tasks::TaskHandle,
    client: Arc<MqttClient>,
    topic_filter: String,
    channel: Option<String>,
    max_messages: usize,
    timeout: std::time::Duration,
) -> anyhow::Result<serde_json::Value> {
    let started = std::time::Instant::now();
    let mut cursor_ms: i64 = client
        .recent(Some(&topic_filter), 1)
        .await
        .first()
        .map(|m| m.received_ms)
        .unwrap_or(0);
    let mut messages: Vec<serde_json::Value> = Vec::new();
    let cancel = handle.cancel_token();
    loop {
        if cancel.is_cancelled() || messages.len() >= max_messages || started.elapsed() >= timeout {
            break;
        }
        let batch = client.recent(Some(&topic_filter), max_messages * 4).await;
        for msg in batch.into_iter().rev() {
            if msg.received_ms <= cursor_ms {
                continue;
            }
            cursor_ms = msg.received_ms;
            let Some(line) = decode_text(&msg, &channel, None) else {
                continue;
            };
            messages.push(serde_json::json!({
                "topic": msg.topic,
                "summary": line.clone(),
                "received_ms": msg.received_ms,
            }));
            handle
                .progress(messages.len() as f64, Some(max_messages as f64), Some(line))
                .await;
            if messages.len() >= max_messages {
                break;
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
        }
    }
    Ok(serde_json::json!({
        "channel_filter": channel,
        "message_count": messages.len(),
        "messages": messages,
    }))
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(MeshtasticMessages),
        Box::new(MeshtasticNodes),
        Box::new(MeshtasticSend),
        Box::new(MeshtasticListen),
        Box::new(MeshtasticStatus),
    ]
}

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "meshtastic"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Meshtastic LoRa mesh — read mesh messages / nodes, send text. v1 transport is \
         MQTT JSON (requires the MQTT family). Serial / TCP / BLE + protobuf decode are a \
         follow-up."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        // Same constraint as MQTT itself: the host has nothing local to
        // probe. The "MQTT actually wired up?" check happens per-call via
        // `require_client` so the error reaches the LLM with the right
        // hint. If we surfaced "Unavailable" here based on env-only
        // signals we'd be guessing.
        crate::skills::SkillCapability::Ready
    }
}
