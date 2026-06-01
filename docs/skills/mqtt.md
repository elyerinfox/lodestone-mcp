# MQTT — `mqtt_publish` / `mqtt_subscribe` / `mqtt_unsubscribe` / `mqtt_recent` / `mqtt_status`

|  |  |
| --- | --- |
| **Module** | [`src/skills/mqtt.rs`](../../src/skills/mqtt.rs) |
| **Tools** | `mqtt_publish`, `mqtt_subscribe`, `mqtt_unsubscribe`, `mqtt_recent`, `mqtt_status` |
| **Network** | outbound to the configured broker |
| **Default** | **off** — gated by `[mqtt].enabled` |
| **Config** | `[mqtt]` in [`config/19-mqtt.toml`](../../config/19-mqtt.toml) |

## What it does
Generic MQTT pub/sub against a configured broker. One persistent connection
(via [`rumqttc`](https://crates.io/crates/rumqttc)), one background event-loop
task, one shared ring buffer of inbound messages. Publish/subscribe handles are
cheap clones; every tool call goes through the same shared `MqttClient` held on
`Lodestone`.

The Meshtastic skill is **layered on top** of this same client — when
`[meshtastic].enabled = true` and `transport = "mqtt"`, the meshtastic family
auto-subscribes to the Meshtastic JSON topic shape on startup and decodes the
recent-buffer entries. There is exactly one MQTT connection for both.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `mqtt_publish` | `topic`, `payload?` (UTF-8) or `payload_base64?` (binary), `qos?` (0/1/2), `retain?` | Publish a message. |
| `mqtt_subscribe` | `topic` (supports `+` / `#` wildcards), `qos?` | Add a subscription. Inbound messages flow into the ring buffer. |
| `mqtt_unsubscribe` | `topic` | Drop a prior subscription. |
| `mqtt_recent` | `topic?` (filter, supports wildcards), `limit?` (default 20, max 200) | Newest-first snapshot of buffered messages; non-UTF-8 payloads rendered as `<binary N bytes>`. |
| `mqtt_status` | — | Broker URL, credentials presence (`<set>` / `<unset>`), default QoS, buffer size + capacity, current subscriptions. |

## Transports
The broker URL scheme picks the transport:
- `tcp://host:port` or `mqtt://host:port` — plain MQTT.
- `tls://host:port` or `mqtts://host:port` — MQTTS over rustls with the default
  webpki root store. Custom CAs aren't wired in for v1; self-host an `tcp://`
  endpoint behind your own TLS terminator if you need one.

## Privacy
`[mqtt].password` is a **secret** (golden rule 11). It is never logged,
returned, or echoed back. `mqtt_status` reports its presence as `<set>` /
`<unset>` only. `[mqtt].username` is shown in the broker summary.

## Buffer
`[mqtt].buffer_size` (default 500) caps the ring buffer. Older messages are
evicted when new ones arrive. Sized per-process — large topic streams may want
a higher cap.

## Capability gating
The family probe is host-agnostic — MQTT is a pure network protocol with
nothing local to check. Wiring state is reflected in `mqtt_status` and in the
per-call failure mode: when `[mqtt].enabled = false` or the broker URL is
empty / invalid, `mqtt_*` tools return a clear error.

## Example uses
- **Light home automation** — `mqtt_publish` to `home/lights/kitchen` with payload `on`.
- **Sensor observation** — `mqtt_subscribe sensors/+/temp` then `mqtt_recent`.
- **Meshtastic substrate** — point `[meshtastic]` at the same broker; reuses the connection.

## See also
[meshtastic.md](meshtastic.md), [golden-rules.md](../golden-rules.md), [tools.md](../tools.md)
