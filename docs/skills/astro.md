# Astronomy — `astro_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/astro.rs`](../../src/skills/astro.rs) |
| **Tools** | `astro_sun`, `astro_moon` |
| **Network** | none — local compute |
| **Default** | **off** — gated by `[astro]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[astro].enabled` via `LODESTONE_ASTRO_ENABLED`. Defaults in [`src/config.rs`](../../src/config.rs). |

## What it does

Solar / lunar position and rise / transit / set for a date and location, plus
moon phase. Pure local computation — no network.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `astro_sun` | `lat`, `lon`, `date?` | Sun position (altitude / azimuth) for now, plus today's sunrise / transit / sunset. `date` defaults to now (UTC). |
| `astro_moon` | `lat`, `lon`, `date?` | Moon position (altitude / azimuth) + rise / set + phase (new / waxing / full / waning + illumination %). |

## Example uses

- **Is the sun up?** — `astro_sun { lat: 47.6, lon: -122.3 }`.
- **Moon phase for an observation night** —
  `astro_moon { lat: 47.6, lon: -122.3, date: "2026-06-15T22:00:00-07:00" }`.

## Notes

- **No atmospheric refraction correction** beyond standard sunrise/sunset
  conventions; close to the horizon, expect ~30' uncertainty.
- **For satellite orbits**, see [`satellite`](satellite.md) (`sat_position`
  via SGP4).

## See also

- [tools.md](../tools.md)
- [skills/satellite.md](satellite.md) — SGP4 propagation, observer
  look-angles.
- [skills/datetime.md](datetime.md) — for timezone math.
