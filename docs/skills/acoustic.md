# Acoustic & underwater — `acoustic_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/acoustic.rs`](../../src/skills/acoustic.rs) |
| **Tools** | `acoustic_sound_speed_water`, `acoustic_sound_speed_air`, `acoustic_snell`, `acoustic_transmission_loss`, `acoustic_sonar_equation` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |

## What it does

Underwater + atmospheric acoustic primitives — sound speed, refraction,
transmission loss, and the sonar equation. Aimed at sonar performance
estimation, hydroacoustic instrumentation work, and the back-of-envelope
"will I hear it" question.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `acoustic_sound_speed_water` | `temp_c`, `salinity_psu`, `depth_m` | Mackenzie 9-term seawater sound-speed formula. |
| `acoustic_sound_speed_air` | `temp_c`, `rh_pct?` | Air sound speed (20.05·√T_K with humidity correction). |
| `acoustic_snell` | `incident_deg`, `c1`, `c2` | Snell's-law refraction; returns the transmitted angle or "totally reflected". |
| `acoustic_transmission_loss` | `range_m`, `frequency_khz`, `geometry?` | Thorp absorption + `spherical` (default, deep) or `cylindrical` (shallow) spreading. |
| `acoustic_sonar_equation` | `sl_db`, `tl_db`, `ts_db`, `nl_db`, `dt_db`, `array_gain_db?` | SE = SL − 2·TL + TS − (NL − AG) − DT. |

## Example uses

- **Surface duct.** `acoustic_sound_speed_water` along a depth profile
  → look for the c(z) gradient sign change to find the duct boundary.
- **Refraction at a thermocline.** `acoustic_snell { incident_deg: 80,
  c1: 1500, c2: 1530 }` → either bent ray angle or total reflection.
- **Detect range.** Iterate `range_m` through `acoustic_transmission_loss`
  + `acoustic_sonar_equation` until SE crosses 0 dB.

## Notes

- The Mackenzie formula is accurate to ±0.07 m/s in the validated range
  (T 2–30 °C, S 25–40 PSU, depth 0–8 000 m).
- Thorp absorption underestimates above ~50 kHz and in fresh water.

## See also

- [tools.md](../tools.md)
- [skills/rf_link.md](rf_link.md) — the RF analogue of these equations.
