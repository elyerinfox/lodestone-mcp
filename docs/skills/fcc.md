# FCC / amateur radio reference — `fcc_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/fcc.rs`](../../src/skills/fcc.rs) |
| **Tools** | `fcc_callsign`, `fcc_amateur_bands`, `fcc_radio_service` |
| **Network** | `fcc_callsign` calls callook.info; the other two are local reference tables |
| **Default** | **on** — gated by `[fcc]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[fcc].enabled` via `LODESTONE_FCC_ENABLED`. Defaults in [`src/config.rs`](../../src/config.rs). |

## What it does

US radio regulatory reference. Three independent tools:

- **`fcc_callsign`** — US amateur callsign lookup via the **keyless**
  callook.info JSON API. Returns the licensee, operator class (Technician /
  General / Amateur Extra), trustee for club calls, grant / expire /
  last-action dates, FRN, mailing address, and grid square. Switched from
  `data.fcc.gov` ULS (HTTP/2-flaky from many networks) to callook.info for
  reliability; non-amateur callsigns (GMRS `WQ*`/`WR*`, commercial,
  broadcast) get a friendly ULS web-search hint instead of an error.
- **`fcc_amateur_bands`** — the full US amateur band plan from **2200m through
  1.25cm** (24 bands total) with per-license-class privileges baked in. `band`
  matches a wavelength label (`40m`, `70cm`), a region (`HF`, `VHF`), or a
  frequency in MHz (`14.250` → 20m); `license_class` filters to Technician /
  General / Amateur Extra.
- **`fcc_radio_service`** — non-amateur personal radio services reference
  (FRS / GMRS / MURS / CB). Channel maps with frequencies and power caps; how
  FRS and GMRS share spectrum (14 shared channels with different power
  limits); license / antenna / repeater rules per service. `service="compare"`
  for the side-by-side table.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `fcc_callsign` | `callsign` | Look up a US amateur callsign via callook.info. |
| `fcc_amateur_bands` | `band?`, `license_class?` | Amateur band plan with per-class privileges. |
| `fcc_radio_service` | `service?`, `channel?` | FRS / GMRS / MURS / CB reference. |

## Example uses

- **Verify a callsign before logging it** —
  `fcc_callsign { callsign: "W1AW" }` → ARRL HQ; Amateur Extra; grid FN31.
- **What can a Tech do on 10m?** —
  `fcc_amateur_bands { band: "10m", license_class: "Technician" }`.
- **Channel lookup** —
  `fcc_amateur_bands { band: 14.250 }` → 20m, all classes, phone segment, CW.
- **FRS vs. GMRS** —
  `fcc_radio_service { service: "compare" }`.
- **GMRS channel** —
  `fcc_radio_service { service: "GMRS", channel: 15 }`.

## Notes

- **Keyless.** callook.info has no rate limit policy; please don't hammer it.
- **US only.** The band-plan and service tables are FCC Part 95 / 97; foreign
  amateur regulations differ.
- **GMRS / commercial callsigns** get a ULS hint rather than a stub answer —
  the FCC's ULS web search has the canonical data.

## See also

- [tools.md](../tools.md)
- [skills/radio.md](radio.md) — RF link math (FSPL, link budget, antenna).
- [skills/sdr.md](sdr.md) — actually receive the bands (`sdr_scan`).
