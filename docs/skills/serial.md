# Serial devices — `serial_ports` / `serial_send` / `serial_read`

|  |  |
| --- | --- |
| **Module** | [`src/skills/serial.rs`](../../src/skills/serial.rs) |
| **Tools** | `serial_ports`, `serial_send`, `serial_read` |
| **Network** | local hardware (direct serial-port I/O) |
| **Default** | off — gated by `[serial]` |
| **Config** | `[serial]` in [`config/16-devices.toml`](../../config/16-devices.toml) |

## What it does
Lists the machine's serial ports and reads/writes raw serial I/O. This is **direct
hardware access**, so the whole family is **off by default** and must be explicitly
enabled. Writing is side-effecting, so `serial_send` routes through the confirmation
[`guard`](../../src/skills/guard.rs). Blocking serial I/O runs on a blocking thread;
per-call `baud`/`timeout_ms` override the `[serial]` defaults.

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `serial_ports` | — | read | List serial ports: name + type (USB VID:PID/product, Bluetooth, PCI, unknown). |
| `serial_send` | `port`, `data`, `baud?`, `confirm?`, `trust?` | **side-effecting** | Write `data` (UTF-8 bytes; append `\n` yourself if needed) to `port` (e.g. `COM3` / `/dev/ttyUSB0`); returns bytes written (confirm first). |
| `serial_read` | `port`, `baud?`, `timeout_ms?`, `max_bytes?` | read | Read until `timeout_ms` elapses or `max_bytes` reached; returns the bytes as text + a hex dump. `max_bytes` default 4096, capped 65536. |

`baud` defaults to `[serial].baud` (9600); `timeout_ms` defaults to
`[serial].timeout_ms` (1000).

## Configuration & gating
- `[serial].enabled` (default `false`, env `LODESTONE_SERIAL_ENABLED`) — exposes
  the `serial_*` tools. **Off by default**; an explicit grant is required, after
  which gating lives in `disabled_by_config`.
- `[serial].baud` (env `LODESTONE_SERIAL_BAUD`) and `[serial].timeout_ms` set the
  per-call defaults. See [`config/16-devices.toml`](../../config/16-devices.toml).
- **Confirmation guard.** `serial_send` is side-effecting: the **first** call writes
  nothing and returns a one-time `confirm` token describing the write (byte count +
  port). Call again with `confirm=<token>` to actually send, or `confirm=<token>`
  plus `trust=true` to stop being asked for `serial_send` for the rest of the
  session. Tokens are single-use and expire after 5 minutes. There is no
  pre-authorize flag for serial — you always confirm or trust. `serial_ports` and
  `serial_read` are read-only.

## Example uses
- **Find a device** — `serial_ports` to see attached ports and their USB ids.
- **Talk to a microcontroller** — `serial_send port=COM3 data="AT\r\n"` (returns a
  token; call again with `confirm=<token>`), then `serial_read port=COM3` to capture
  the reply.

## See also
[tools.md](../tools.md), [golden-rules.md](../golden-rules.md)
