# Software-defined radio — `sdr_devices`, `sdr_scan`

|  |  |
| --- | --- |
| **Module** | [`src/skills/sdr.rs`](../../src/skills/sdr.rs) |
| **Tools** | `sdr_devices`, `sdr_scan` |
| **Network** | none (shells out to local SDR CLI tools) |
| **Default** | **off** — gated by `[sdr]` |
| **Config** | `[sdr]` in [`config/16-devices.toml`](../../config/16-devices.toml) |

## What it does
Lists attached software-defined radios and sweeps the RF spectrum, by shelling out
to the standard CLI tools — `rtl_test`/`hackrf_info` for discovery and `rtl_power`
for a power sweep. Off by default; needs the tools installed and hardware attached.
**Receive-only**: there is deliberately no transmit path. If a tool isn't on
`PATH`, the error says so.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `sdr_devices` | — | Probe for RTL-SDR (`rtl_test`) and HackRF (`hackrf_info`) devices. |
| `sdr_scan` | `start_mhz`, `end_mhz`, `bin_khz?`, `top?` | Single `rtl_power` sweep; reports the strongest bins (frequency + dB). |

- `bin_khz` sets the resolution (default 100 kHz); `top` caps how many of the
  loudest bins are returned (default 15, max 100).
- Frequencies must be within 0–6000 MHz and `start_mhz < end_mhz`. Sweeps are
  bounded by a timeout and the child process is killed if it overruns.

## Example uses
- **What's plugged in** — `sdr_devices`.
- **FM broadcast band** — `sdr_scan { start_mhz: 88, end_mhz: 108 }` → the
  strongest stations.
- **ISM / 433 MHz** — `sdr_scan { start_mhz: 433, end_mhz: 434, bin_khz: 5 }`.

## Notes
- Receive-only by design — transmission (e.g. `hackrf_transfer -t`) is intentionally
  not exposed.
- A wide range at fine resolution takes longer; narrow the range or coarsen `bin_khz`.

## See also
[tools.md](../tools.md)
