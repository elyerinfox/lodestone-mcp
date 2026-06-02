//! MQTT pub/sub skill — one persistent connection to a configured broker.
//! Off by default (`[mqtt].enabled`), network-touching. The broker URL
//! scheme picks the transport: `tcp://host:port` (plain MQTT) or
//! `tls://host:port` (MQTTS over rustls).
//!
//! Architecture: a single background event-loop task owns the
//! `rumqttc::EventLoop`; an `Arc<MqttClient>` carrying the cheap
//! [`rumqttc::AsyncClient`] handle plus a shared ring buffer of recent
//! messages is held on [`crate::Lodestone`] so every tool call sees the
//! same state. Tools never own the connection.
//!
//! Privacy: the broker `password` is a secret (golden rule 11). It is
//! redacted to `<set>` / `<unset>` in every status / error string and
//! never logged. The broker URL **without** userinfo is fair to surface.
//!
//! Companion: the meshtastic family reads Meshtastic JSON messages out
//! of the same recent-message buffer when configured with
//! `transport = "mqtt"`. MQTT here is the substrate; meshtastic is the
//! decoder layered on top.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Packet, QoS};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::config;
use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

/// One buffered message kept in the ring-buffer that
/// `mqtt_recent` / `meshtastic_messages` read from.
#[derive(Debug, Clone)]
pub struct MqttMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    /// Unix milliseconds the message was received.
    pub received_ms: i64,
    pub qos: u8,
    pub retain: bool,
}

/// Shared MQTT client + state. Stored as `Option<Arc<MqttClient>>` on
/// [`crate::Lodestone`]: `None` when `[mqtt].enabled = false` or when
/// the broker URL is empty (capability check rejects the call so the
/// dispatch never enters a tool body without a client).
#[derive(Debug)]
pub struct MqttClient {
    pub(crate) client: AsyncClient,
    pub(crate) buffer: Mutex<VecDeque<MqttMessage>>,
    pub(crate) subscriptions: Mutex<HashSet<String>>,
    pub(crate) broker_display: String,
    pub(crate) username_set: bool,
    pub(crate) password_set: bool,
    pub(crate) default_qos: u8,
    pub(crate) buffer_capacity: usize,
}

impl MqttClient {
    /// Build the MqttOptions, kick off the background event loop, and
    /// return the shared client. Auto-subscribes to any topics listed
    /// in `[mqtt].auto_subscribe`.
    pub async fn start(cfg: &config::Mqtt) -> Result<Arc<Self>> {
        if cfg.broker.trim().is_empty() {
            return Err(anyhow!(
                "[mqtt].broker is empty — set tcp://host:port or tls://host:port"
            ));
        }
        let (host, port, use_tls) = parse_broker(&cfg.broker)?;

        let client_id = if cfg.client_id.trim().is_empty() {
            // Random-ish id derived from the broker + a process-lifetime
            // counter so it's stable across reconnects but doesn't clash
            // with other lodestone instances pointed at the same broker.
            format!(
                "lodestone-{:08x}",
                std::process::id().wrapping_mul(0x9E37_79B9)
            )
        } else {
            cfg.client_id.clone()
        };
        let mut opts = MqttOptions::new(client_id, host.clone(), port);
        opts.set_keep_alive(Duration::from_secs(cfg.keep_alive_secs.max(5) as u64));
        if use_tls {
            // Use the platform's native TLS stack (Schannel on Windows,
            // Secure Transport on macOS, OpenSSL on Linux) so OS-vendor
            // CVE patches apply. Users who need a custom CA can self-host
            // a `tcp://` broker behind their own TLS terminator for v1.
            opts.set_transport(rumqttc::Transport::tls_with_config(
                rumqttc::TlsConfiguration::Native,
            ));
        }
        if !cfg.username.is_empty() {
            opts.set_credentials(&cfg.username, &cfg.password);
        }
        let (client, eventloop) = AsyncClient::new(opts, 64);

        let buffer_capacity = cfg.buffer_size.max(16);
        let state = Arc::new(MqttClient {
            client: client.clone(),
            buffer: Mutex::new(VecDeque::with_capacity(buffer_capacity)),
            subscriptions: Mutex::new(HashSet::new()),
            broker_display: format!(
                "{scheme}{host}:{port}",
                scheme = if use_tls { "tls://" } else { "tcp://" }
            ),
            username_set: !cfg.username.is_empty(),
            password_set: !cfg.password.is_empty(),
            default_qos: cfg.default_qos.min(2),
            buffer_capacity,
        });

        // Background event loop — polls forever, pushing inbound
        // publishes into the buffer. Reconnects are handled by rumqttc.
        tokio::spawn(run_event_loop(state.clone(), eventloop));

        // Auto-subscriptions.
        for topic in &cfg.auto_subscribe {
            if let Err(e) = state
                .subscribe_internal(topic, qos_from_u8(state.default_qos))
                .await
            {
                tracing::warn!(target: "mqtt", topic, error = %e, "auto-subscribe failed");
            }
        }
        Ok(state)
    }

    pub fn broker(&self) -> &str {
        &self.broker_display
    }

    pub async fn publish(
        &self,
        topic: &str,
        qos: u8,
        retain: bool,
        payload: Vec<u8>,
    ) -> Result<()> {
        self.client
            .publish(topic, qos_from_u8(qos), retain, payload)
            .await
            .map_err(|e| anyhow!("publish failed: {e}"))
    }

    pub async fn subscribe(&self, topic: &str, qos: u8) -> Result<()> {
        self.subscribe_internal(topic, qos_from_u8(qos)).await
    }

    async fn subscribe_internal(&self, topic: &str, qos: QoS) -> Result<()> {
        self.client
            .subscribe(topic, qos)
            .await
            .map_err(|e| anyhow!("subscribe failed: {e}"))?;
        self.subscriptions.lock().await.insert(topic.to_string());
        Ok(())
    }

    pub async fn unsubscribe(&self, topic: &str) -> Result<()> {
        self.client
            .unsubscribe(topic)
            .await
            .map_err(|e| anyhow!("unsubscribe failed: {e}"))?;
        self.subscriptions.lock().await.remove(topic);
        Ok(())
    }

    /// Snapshot the most recent messages, optionally filtered by an
    /// MQTT-style topic glob. Returns newest-first.
    pub async fn recent(&self, topic_filter: Option<&str>, limit: usize) -> Vec<MqttMessage> {
        let buf = self.buffer.lock().await;
        let mut out: Vec<MqttMessage> = Vec::with_capacity(limit);
        for msg in buf.iter().rev() {
            if let Some(filter) = topic_filter {
                if !topic_matches(filter, &msg.topic) {
                    continue;
                }
            }
            out.push(msg.clone());
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    pub async fn subscribed_topics(&self) -> Vec<String> {
        let mut out: Vec<String> = self.subscriptions.lock().await.iter().cloned().collect();
        out.sort();
        out
    }

    pub async fn buffer_size(&self) -> usize {
        self.buffer.lock().await.len()
    }
}

/// Push an inbound publish into the ring buffer, evicting the oldest
/// when capacity is reached.
async fn record(state: &MqttClient, topic: String, payload: Vec<u8>, qos: u8, retain: bool) {
    let mut buf = state.buffer.lock().await;
    if buf.len() >= state.buffer_capacity {
        buf.pop_front();
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    buf.push_back(MqttMessage {
        topic,
        payload,
        received_ms: now_ms,
        qos,
        retain,
    });
}

async fn run_event_loop(state: Arc<MqttClient>, mut eventloop: EventLoop) {
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::Publish(p))) => {
                let qos = match p.qos {
                    QoS::AtMostOnce => 0,
                    QoS::AtLeastOnce => 1,
                    QoS::ExactlyOnce => 2,
                };
                record(&state, p.topic, p.payload.to_vec(), qos, p.retain).await;
            }
            Ok(_) => {}
            Err(e) => {
                // rumqttc handles reconnects internally; log and back off.
                tracing::warn!(target: "mqtt", error = %e, "event loop error; rumqttc will retry");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/// MQTT wildcard match: `+` matches one level, `#` matches zero or more
/// trailing levels (per MQTT spec — `home/#` matches `home`, `home/x`,
/// `home/x/y`, …). Exact equality returns true.
pub(crate) fn topic_matches(filter: &str, topic: &str) -> bool {
    if filter == topic {
        return true;
    }
    let fparts: Vec<&str> = filter.split('/').collect();
    let tparts: Vec<&str> = topic.split('/').collect();
    let mut fi = 0;
    let mut ti = 0;
    loop {
        // `#` swallows whatever's left (including nothing). MQTT spec
        // requires `#` to be the last segment of the filter; we trust
        // that here — bogus filters with `#` mid-string degrade to
        // "matches anything from this position on" which is the same
        // behavior most brokers tolerate.
        if fi < fparts.len() && fparts[fi] == "#" {
            return true;
        }
        if fi == fparts.len() && ti == tparts.len() {
            return true;
        }
        if fi >= fparts.len() || ti >= tparts.len() {
            return false;
        }
        match fparts[fi] {
            "+" => {
                fi += 1;
                ti += 1;
            }
            seg if seg == tparts[ti] => {
                fi += 1;
                ti += 1;
            }
            _ => return false,
        }
    }
}

fn parse_broker(url: &str) -> Result<(String, u16, bool)> {
    let url = url.trim();
    let (scheme, rest) = url
        .split_once("://")
        .with_context(|| format!("missing scheme in broker URL: {url:?}"))?;
    let use_tls = match scheme.to_ascii_lowercase().as_str() {
        "tcp" | "mqtt" => false,
        "tls" | "ssl" | "mqtts" => true,
        other => return Err(anyhow!("unsupported broker scheme {other:?}")),
    };
    let (host, port) = rest
        .rsplit_once(':')
        .with_context(|| format!("broker URL missing port: {url:?}"))?;
    let port: u16 = port
        .parse()
        .with_context(|| format!("broker URL port not numeric: {port:?}"))?;
    Ok((host.to_string(), port, use_tls))
}

fn qos_from_u8(v: u8) -> QoS {
    match v {
        0 => QoS::AtMostOnce,
        2 => QoS::ExactlyOnce,
        _ => QoS::AtLeastOnce,
    }
}

fn require_client(server: &crate::Lodestone) -> Result<&Arc<MqttClient>, McpError> {
    server.mqtt.as_ref().ok_or_else(|| {
        invalid(
            "MQTT client not initialized — set [mqtt].enabled = true and configure [mqtt].broker",
        )
    })
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PublishArgs {
    /// Topic to publish to (e.g. `home/lights/kitchen`).
    topic: String,
    /// Message payload. UTF-8 text. For binary you'll need `payload_base64`.
    #[serde(default)]
    payload: Option<String>,
    /// Base64-encoded binary payload (alternative to `payload`).
    #[serde(default)]
    payload_base64: Option<String>,
    /// QoS 0 / 1 / 2. Omit for the `[mqtt].default_qos` default.
    #[serde(default)]
    qos: Option<u8>,
    /// Broker retains the last message on this topic (delivered to new subscribers).
    #[serde(default)]
    retain: Option<bool>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, stop asking for `mqtt_publish` for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

pub struct MqttPublish;
impl Skill for MqttPublish {
    fn name(&self) -> &'static str {
        "mqtt_publish"
    }
    fn description(&self) -> &'static str {
        "Publish a message to an MQTT topic via the configured broker ([mqtt]). \
        Pass `payload` for UTF-8 text or `payload_base64` for binary; `qos` (0/1/2) \
        and `retain` are optional. **Side-effecting** — the first call returns a \
        confirmation token and does nothing; call again with `confirm=<token>` to \
        publish (or `confirm + trust=true`). `[mqtt].allow_destructive=true` \
        pre-authorizes. Publishes can drive IoT actuators / smart-home devices / \
        anything subscribed to the topic — be specific in the prompt."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PublishArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use crate::skills::guard::Decision;
            let (server, args) = ctx.parse::<PublishArgs>()?;
            let client = require_client(server)?;
            let payload: Vec<u8> = if let Some(b64) = args.payload_base64.as_deref() {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD
                    .decode(b64.trim())
                    .map_err(|e| invalid(format!("payload_base64 is not valid base64: {e}")))?
            } else {
                args.payload.unwrap_or_default().into_bytes()
            };
            let qos = args.qos.unwrap_or(client.default_qos).min(2);
            let retain = args.retain.unwrap_or(false);
            let summary = format!(
                "publish {} byte(s) to {} on {} (qos={qos}, retain={retain})",
                payload.len(),
                args.topic,
                client.broker()
            );
            if let Decision::Challenge(msg) = server.guard.check(
                "mqtt_publish",
                "mqtt_publish",
                server.cfg.mqtt.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            client
                .publish(&args.topic, qos, retain, payload)
                .await
                .map_err(internal)?;
            Ok(text_result(format!(
                "Published to {} (qos={qos}, retain={retain}) on {}.",
                args.topic,
                client.broker()
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "First call returns a confirmation token",
                args: r#"{"topic": "home/lights/kitchen", "payload": "on"}"#,
                note: Some(
                    "Returns a token; call again with `confirm=<token>` to actually publish.",
                ),
            },
            SkillExample {
                title: "Retained value, confirmed",
                args: r#"{"topic": "home/lights/kitchen", "payload": "on", "retain": true, "confirm": "abc123"}"#,
                note: Some("Broker stores the message and delivers it to future subscribers."),
            },
            SkillExample {
                title: "Binary payload via base64",
                args: r#"{"topic": "devices/firmware", "payload_base64": "AAECAw==", "qos": 1, "confirm": "abc123"}"#,
                note: Some("Use `payload_base64` whenever the bytes aren't UTF-8 text."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Drive an MQTT-controlled actuator (smart bulb, relay, valve) by writing to its command topic.",
            "Seed a new retained value so future subscribers see initial state.",
            "Inject a test message onto a topic to exercise a subscriber pipeline.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SubscribeArgs {
    /// Topic filter — supports MQTT wildcards `+` (one level) and `#` (rest).
    /// e.g. `sensors/+/temp` or `home/#`.
    topic: String,
    /// QoS 0 / 1 / 2. Omit for the `[mqtt].default_qos` default.
    #[serde(default)]
    qos: Option<u8>,
}

pub struct MqttSubscribe;
impl Skill for MqttSubscribe {
    fn name(&self) -> &'static str {
        "mqtt_subscribe"
    }
    fn description(&self) -> &'static str {
        "Subscribe to an MQTT topic filter (supports `+` / `#` wildcards). Inbound \
        messages flow into a process-wide ring buffer (size `[mqtt].buffer_size`); \
        read them back with `mqtt_recent`. Returns the current subscription list."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SubscribeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SubscribeArgs>()?;
            let client = require_client(server)?;
            let qos = args.qos.unwrap_or(client.default_qos).min(2);
            client.subscribe(&args.topic, qos).await.map_err(internal)?;
            let subs = client.subscribed_topics().await;
            Ok(text_result(format!(
                "Subscribed to {} (qos={qos}). Active subscriptions ({}):\n  {}",
                args.topic,
                subs.len(),
                subs.join("\n  ")
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Exact topic",
                args: r#"{"topic": "home/lights/kitchen"}"#,
                note: None,
            },
            SkillExample {
                title: "Single-level wildcard",
                args: r#"{"topic": "sensors/+/temp", "qos": 1}"#,
                note: Some("`+` matches exactly one path segment."),
            },
            SkillExample {
                title: "Multi-level wildcard at the tail",
                args: r#"{"topic": "home/#"}"#,
                note: Some("`#` swallows the rest of the topic; must be the last segment."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Start buffering messages on a topic so a later `mqtt_recent` call has data to read.",
            "Add a new wildcard filter while the broker stays connected.",
            "Set up monitoring before publishing a probe with `mqtt_publish`.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UnsubscribeArgs {
    /// Topic filter to drop (must match an earlier subscription string exactly).
    topic: String,
}

pub struct MqttUnsubscribe;
impl Skill for MqttUnsubscribe {
    fn name(&self) -> &'static str {
        "mqtt_unsubscribe"
    }
    fn description(&self) -> &'static str {
        "Drop an MQTT subscription previously added by `mqtt_subscribe`. The topic \
        must match the original subscription string exactly."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<UnsubscribeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<UnsubscribeArgs>()?;
            let client = require_client(server)?;
            client.unsubscribe(&args.topic).await.map_err(internal)?;
            let subs = client.subscribed_topics().await;
            Ok(text_result(format!(
                "Unsubscribed from {}. {} subscription(s) remain.",
                args.topic,
                subs.len()
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Drop an exact topic",
                args: r#"{"topic": "home/lights/kitchen"}"#,
                note: None,
            },
            SkillExample {
                title: "Drop a wildcard filter",
                args: r#"{"topic": "sensors/+/temp"}"#,
                note: Some("Must match the original `mqtt_subscribe` filter string verbatim."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Stop buffering a noisy topic once you've extracted what you need.",
            "Clean up the subscription list before swapping in a different filter shape.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RecentArgs {
    /// Optional topic filter (MQTT wildcards `+` and `#` allowed). Omit to return
    /// every buffered message regardless of topic.
    #[serde(default)]
    topic: Option<String>,
    /// Cap on the number of messages returned (default 20, max 200; newest first).
    #[serde(default)]
    limit: Option<usize>,
}

pub struct MqttRecent;
impl Skill for MqttRecent {
    fn name(&self) -> &'static str {
        "mqtt_recent"
    }
    fn description(&self) -> &'static str {
        "Return recent MQTT messages from the in-memory ring buffer (newest first). \
        Optionally filter by topic (supports `+` / `#` wildcards). Payloads are \
        rendered as UTF-8 with a `<binary N bytes>` placeholder for non-text. \
        Buffer size is `[mqtt].buffer_size`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RecentArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<RecentArgs>()?;
            let client = require_client(server)?;
            let limit = args.limit.unwrap_or(20).clamp(1, 200);
            let msgs = client.recent(args.topic.as_deref(), limit).await;
            if msgs.is_empty() {
                return Ok(text_result(
                    "No buffered messages match (yet). Subscribe with `mqtt_subscribe` \
                     and wait for traffic.",
                ));
            }
            let mut out = format!("{} message(s):", msgs.len());
            for m in msgs {
                let body = match std::str::from_utf8(&m.payload) {
                    Ok(s) if !s.contains('\0') => s.to_string(),
                    _ => format!("<binary {} bytes>", m.payload.len()),
                };
                out.push_str(&format!(
                    "\n[{}] {} (qos={}, retain={})\n  {}",
                    m.received_ms, m.topic, m.qos, m.retain, body
                ));
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Whole buffer, default 20",
                args: r#"{}"#,
                note: Some("Returns the 20 newest buffered messages across all topics."),
            },
            SkillExample {
                title: "Filter by topic wildcard",
                args: r#"{"topic": "sensors/+/temp", "limit": 50}"#,
                note: Some("Wildcards `+` / `#` follow MQTT semantics."),
            },
            SkillExample {
                title: "Tail one specific topic",
                args: r#"{"topic": "home/lights/kitchen", "limit": 5}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Sample what's been arriving on a subscription without spawning a watcher task.",
            "Verify a publish actually landed on the broker by reading it back.",
            "Inspect a few recent messages to figure out the payload shape before parsing.",
        ]
    }
}

pub struct MqttStatus;
impl Skill for MqttStatus {
    fn name(&self) -> &'static str {
        "mqtt_status"
    }
    fn description(&self) -> &'static str {
        "Report MQTT connection state: broker URL, whether credentials are set \
        (`<set>` / `<unset>`, never the value), active subscriptions, buffer size. \
        Credentials are never echoed."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let client = require_client(ctx.server)?;
            let subs = client.subscribed_topics().await;
            let buf_len = client.buffer_size().await;
            let mut out = format!(
                "MQTT\n  broker: {}\n  username: {}\n  password: {}\n  default_qos: {}\n  \
                 buffer: {} / {}\n  subscriptions ({}):",
                client.broker(),
                if client.username_set {
                    "<set>"
                } else {
                    "<unset>"
                },
                if client.password_set {
                    "<set>"
                } else {
                    "<unset>"
                },
                client.default_qos,
                buf_len,
                client.buffer_capacity,
                subs.len()
            );
            for s in subs {
                out.push_str(&format!("\n    {s}"));
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Print the current MQTT wiring",
                args: r#"{}"#,
                note: Some("Shows broker URL, credential presence, default QoS, buffer usage, and subscriptions."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Confirm the broker URL and credential state without leaking secrets.",
            "Check the active subscription list before deciding what to publish or recent.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListenArgs {
    /// Topic filter to watch (MQTT wildcards `+` / `#` allowed). Auto-
    /// subscribes if not already active.
    topic: String,
    /// Stop after collecting this many matching messages. Default 50, max 1000.
    #[serde(default)]
    max_messages: Option<usize>,
    /// Stop after this many seconds even if fewer messages arrived. Default 60, max 3600.
    #[serde(default)]
    timeout_secs: Option<u64>,
}

pub struct MqttListen;
impl Skill for MqttListen {
    fn name(&self) -> &'static str {
        "mqtt_listen"
    }
    fn description(&self) -> &'static str {
        "Spawn an async task that watches an MQTT topic filter and streams \
        matching messages via `notifications/progress` (one per inbound \
        publish, correlated by the caller's `_meta.progressToken`). Returns a \
        `task_id` immediately; lifecycle changes push \
        `notifications/tasks/status`. Fetch buffered messages via \
        `tasks_result`, cancel via `tasks_cancel`. Stops at `max_messages`, \
        `timeout_secs`, or cancellation."
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
            let max_messages = args.max_messages.unwrap_or(50).clamp(1, 1000);
            let timeout =
                std::time::Duration::from_secs(args.timeout_secs.unwrap_or(60).clamp(1, 3600));
            // Auto-subscribe (no-op if already subscribed at this topic).
            client
                .subscribe(&args.topic, client.default_qos)
                .await
                .map_err(crate::internal)?;
            let topic_filter = args.topic.clone();
            let label = format!("mqtt_listen {topic_filter}");
            let runtime = server.task_runtime.clone();
            let runtime_for_observers = runtime.clone();
            let task_id = runtime
                .spawn("mqtt_listen", label, move |handle| async move {
                    listen_body(handle, client, topic_filter, max_messages, timeout).await
                })
                .await;
            // Wire the caller's progressToken (if any) and the peer to the
            // task so progress / status notifications fan out.
            if let (Some(peer), Some(token)) = (peer.clone(), token) {
                runtime_for_observers
                    .observe_progress(&task_id, peer, token)
                    .await;
            }
            if let Some(peer) = peer {
                runtime_for_observers.observe_status(&task_id, peer).await;
            }
            Ok(text_result(format!(
                "Listening on {} (task_id={}). Watch for `notifications/progress` (per message) and `notifications/tasks/status` (completion). Fetch the buffered messages with `tasks_result {{\"task_id\":\"{}\"}}`.",
                args.topic, task_id, task_id
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Watch a wildcard until 50 messages or 60s",
                args: r#"{"topic": "sensors/+/temp"}"#,
                note: Some(
                    "Returns a task_id immediately; results stream via `notifications/progress`.",
                ),
            },
            SkillExample {
                title: "Bounded watch with explicit limits",
                args: r#"{"topic": "home/#", "max_messages": 10, "timeout_secs": 30}"#,
                note: Some("Stops early once either bound is hit; cancel via `tasks_cancel`."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Stream live MQTT messages to the model without polling `mqtt_recent` repeatedly.",
            "Capture a fixed batch of inbound traffic for analysis, with a hard timeout.",
            "Wait for a specific publish that's expected to arrive after another action.",
        ]
    }
}

/// Watch a topic filter on the shared MQTT client and emit one progress
/// update per inbound message. Polls the ring buffer (200 ms cadence)
/// using `received_ms` as a high-water cursor — cheap, no extra
/// channel plumbing on `MqttClient`. Stops on `max_messages`, timeout,
/// or cancellation.
async fn listen_body(
    handle: crate::tasks::TaskHandle,
    client: Arc<MqttClient>,
    topic_filter: String,
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
    let mut collected: Vec<serde_json::Value> = Vec::new();
    let cancel = handle.cancel_token();
    loop {
        if cancel.is_cancelled() {
            break;
        }
        if collected.len() >= max_messages {
            break;
        }
        if started.elapsed() >= timeout {
            break;
        }
        // Pull up to max_messages each tick, take those past cursor.
        let batch = client.recent(Some(&topic_filter), max_messages).await;
        // recent() returns newest-first; flip to oldest-first for stable ordering.
        for msg in batch.into_iter().rev() {
            if msg.received_ms <= cursor_ms {
                continue;
            }
            cursor_ms = msg.received_ms;
            let body_preview = match std::str::from_utf8(&msg.payload) {
                Ok(s) if !s.contains('\0') => s.chars().take(140).collect::<String>(),
                _ => format!("<binary {} bytes>", msg.payload.len()),
            };
            let entry = serde_json::json!({
                "topic": msg.topic,
                "payload": body_preview,
                "qos": msg.qos,
                "retain": msg.retain,
                "received_ms": msg.received_ms,
            });
            collected.push(entry);
            handle
                .progress(
                    collected.len() as f64,
                    Some(max_messages as f64),
                    Some(format!("{}: {}", msg.topic, body_preview)),
                )
                .await;
            if collected.len() >= max_messages {
                break;
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {}
        }
    }
    Ok(serde_json::json!({
        "topic_filter": topic_filter,
        "message_count": collected.len(),
        "messages": collected,
    }))
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(MqttPublish),
        Box::new(MqttSubscribe),
        Box::new(MqttUnsubscribe),
        Box::new(MqttRecent),
        Box::new(MqttListen),
        Box::new(MqttStatus),
    ]
}

/// Family probe: needs [mqtt].enabled = true plus a non-empty,
/// parseable broker URL.
pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "mqtt"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "MQTT pub/sub client — publish / subscribe / read recent messages against a \
         configured broker. Off by default; the meshtastic skill rides on top when \
         a Meshtastic node is bridged to the same broker."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        // Host-level: nothing to probe. MQTT is a pure network protocol — no
        // binary, no kernel feature. The broker URL lives in config and is
        // validated at startup in `MqttClient::start()`; if the URL is empty
        // or unreachable, `Lodestone.mqtt` stays `None` and each tool's
        // `require_client` fails with an actionable message that the
        // dispatch wrapper surfaces to the LLM. Operator-visible config-vs-
        // wired state is reflected in `mqtt_status`.
        crate::skills::SkillCapability::Ready
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_broker, topic_matches};

    #[test]
    fn wildcard_plus_matches_one_level() {
        assert!(topic_matches("sensors/+/temp", "sensors/kitchen/temp"));
        assert!(!topic_matches(
            "sensors/+/temp",
            "sensors/kitchen/inside/temp"
        ));
        assert!(!topic_matches("sensors/+/temp", "sensors/kitchen/humidity"));
    }

    #[test]
    fn wildcard_hash_matches_rest() {
        assert!(topic_matches("home/#", "home"));
        assert!(topic_matches("home/#", "home/kitchen/lights"));
        assert!(topic_matches("#", "anything/at/all"));
        assert!(!topic_matches("home/#", "office/lights"));
    }

    #[test]
    fn exact_topics_match() {
        assert!(topic_matches("a/b/c", "a/b/c"));
        assert!(!topic_matches("a/b/c", "a/b"));
        assert!(!topic_matches("a/b", "a/b/c"));
    }

    #[test]
    fn meshtastic_filter_against_real_topic() {
        // A typical Meshtastic JSON publish — the filter shape the
        // meshtastic family auto-subscribes with should match.
        assert!(topic_matches(
            "msh/+/2/json/#",
            "msh/US/2/json/LongFast/!11223344",
        ));
        assert!(!topic_matches(
            "msh/+/2/json/#",
            "msh/US/3/json/LongFast/!11223344",
        ));
    }

    #[test]
    fn broker_url_schemes() {
        assert_eq!(
            parse_broker("tcp://broker.example:1883").unwrap(),
            ("broker.example".into(), 1883, false)
        );
        assert_eq!(
            parse_broker("tls://broker.example:8883").unwrap(),
            ("broker.example".into(), 8883, true)
        );
        assert_eq!(parse_broker("mqtts://h:1").unwrap(), ("h".into(), 1, true));
        assert!(parse_broker("broker.example:1883").is_err());
        assert!(parse_broker("http://x:1").is_err());
        assert!(parse_broker("tcp://broker.example").is_err());
    }
}
