# Meshtastic — `meshtastic_messages` / `meshtastic_nodes` / `meshtastic_send` / `meshtastic_status`

|  |  |
| --- | --- |
| **Module** | [`src/skills/meshtastic.rs`](../../src/skills/meshtastic.rs) |
| **Tools** | `meshtastic_messages`, `meshtastic_nodes`, `meshtastic_send`, `meshtastic_status` |
| **Network** | through the configured MQTT broker (`[mqtt]`) |
| **Default** | **off** — gated by `[meshtastic].enabled` |
| **Config** | `[meshtastic]` in [`config/20-meshtastic.toml`](../../config/20-meshtastic.toml) |

## What it does
Reads and writes [Meshtastic](https://meshtastic.org) LoRa mesh traffic. v1
transport is the **MQTT JSON topic format** Meshtastic firmware emits when its
MQTT module has `json_enabled = true` and a broker is configured. A node
bridged to that broker decodes / retransmits mesh frames on LoRa.

The skill is a **decoder layer on top of [`[mqtt]`](mqtt.md)** — there is no
second connection or buffer; meshtastic auto-subscribes to
`<root>/+/2/json/#` on the shared MqttClient and reads matched messages out of
the same ring buffer.

Serial / TCP / BLE transports plus protobuf decode are deferred to a follow-up;
they require the upstream Meshtastic `.proto` files and a `prost`-based build.
The JSON-over-MQTT path covers the most common bridge configuration today.

## Topic shape
- Inbound: `<root>/<region>/2/json/<channel>/<node>` carrying a JSON envelope
  with `from`, `to`, `channel`, `payload`, `rssi`, `snr`, `timestamp`, `type`.
- Outbound (sent by `meshtastic_send`):
  `<root>/<region>/2/json/<channel>/<to>` with
  `{"type":"sendtext", "channel":..., "payload":..., "to":...}`.

`<root>` defaults to `msh` (Meshtastic firmware default).

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `meshtastic_messages` | `channel?`, `from?` (`!11223344` form), `limit?` | Recent text messages decoded from the buffer (newest first). |
| `meshtastic_nodes` | — | Nodes recently heard — id, long/short name (when present), last RSSI / SNR, last-seen ms-ago. |
| `meshtastic_send` | `text`, `channel?`, `region?`, `to?` (default `^all`) | Publish a text message onto the mesh through the bridging node. Caps at ~200 bytes (single LoRa frame). |
| `meshtastic_status` | — | Transport, topic root, defaults, MQTT-wiring state, mesh-matching buffered count. |

## Requirements
- A Meshtastic node forwarding to an MQTT broker with `json_enabled = true`
  (set in the device's `MQTT` module). The broker can be the public one
  (`mqtt.meshtastic.org`) or self-hosted.
- `[mqtt].enabled = true` with `[mqtt].broker` pointing at the same broker.

## Capability gating
The family probe is `Ready` — there's nothing host-local to check. The
"is MQTT actually wired up?" gate happens per-call: each tool calls
`require_client` which returns an actionable error if `Lodestone.mqtt` is
`None` (i.e. `[mqtt]` disabled or the broker never connected). `meshtastic_status`
reports this state plainly.

## Auto-subscribe
With `[meshtastic].auto_subscribe = true` (the default), the family
auto-subscribes to `<root>/+/2/json/#` at startup so the buffer fills without
an explicit `mqtt_subscribe`. Turn it off if you want to subscribe to a
narrower filter yourself via `mqtt_subscribe`.

## Limits
- Outbound `text` is capped at 220 bytes to fit a single LoRa frame.
- Coverage of `meshtastic_nodes` is "whatever has crossed the buffer since
  startup" — there is no persistent node directory in v1.
- Payload decryption (per-channel PSK) isn't handled here; the bridging node
  emits decoded JSON. Encrypted mesh-only payloads stay opaque.

## See also
[mqtt.md](mqtt.md), [Meshtastic JSON docs](https://meshtastic.org/docs/software/integrations/mqtt/),
[golden-rules.md](../golden-rules.md), [tools.md](../tools.md)
