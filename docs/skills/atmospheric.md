# Atmospheric models — `atm_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/atmospheric.rs`](../../src/skills/atmospheric.rs) |
| **Tools** | `atm_isa`, `atm_density_altitude`, `atm_dewpoint`, `atm_wbgt`, `atm_space_weather_kp` |
| **Network** | one live feed: `atm_space_weather_kp` → NOAA SWPC |
| **Default** | on; gateable via `[tools]` |

## What it does

Standard-atmosphere and humidity helpers for aviation / instrumentation work
plus a live planetary K-index for HF-propagation and auroral context.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `atm_isa` | `altitude_m` | US-Standard-Atmosphere-1976 ISA: temperature (K), pressure (Pa), density (kg/m³) at geopotential altitude (≤ 86 km). |
| `atm_density_altitude` | `pressure_pa`, `temp_c`, `dewpoint_c?` | Density altitude (m); applies humidity correction when `dewpoint_c` is supplied. |
| `atm_dewpoint` | `temp_c`, `rh_pct` | Dewpoint via Magnus-Tetens formula. |
| `atm_wbgt` | `temp_c`, `rh_pct` | Wet-bulb global temperature via Stull's approximation (no solar / wind input). |
| `atm_space_weather_kp` | — | Last-24h planetary K-index from `services.swpc.noaa.gov` (3-hour resolution). |

## Example uses

- **Density-altitude for take-off perf.** Field `(pressure_pa, temp_c,
  dewpoint_c)` → `atm_density_altitude` gives the equivalent altitude an
  unaspirated engine "feels".
- **HF propagation today?** `atm_space_weather_kp` — Kp ≥ 5 means a
  geomagnetic disturbance and degraded HF.
- **Cabin air check.** `atm_dewpoint` from cabin temp/humidity to pick a
  condensation threshold.

## Notes

- **`atm_isa`** is geopotential altitude, **not** geometric — for the
  difference at high altitudes apply the standard correction (≈ 0.3 % at
  86 km).
- **`atm_wbgt`** is the indoor / shaded approximation. A full outdoor WBGT
  requires globe temperature + wind, not modelled here.

## See also

- [tools.md](../tools.md)
- [skills/weather.md](weather.md) — point forecasts that pair well with this.
- [skills/rf_link.md](rf_link.md) — atmospheric absorption (P.676) for RF.
