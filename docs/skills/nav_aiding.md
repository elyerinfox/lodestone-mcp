# Navigation aiding — `nav_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/nav_aiding.rs`](../../src/skills/nav_aiding.rs) |
| **Tools** | `nav_dop`, `nav_klobuchar`, `nav_saastamoinen`, `nav_ecef_to_enu`, `nav_imu_drift` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |

## What it does

GNSS / IMU aiding helpers — geometry-of-satellites DOP, broadcast
ionospheric correction, tropospheric delay, ECEF-to-ENU frame conversion,
and an IMU drift error budget. These are the small pieces a model needs
to reason about why a fix is degraded or how much an inertial coast
costs.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `nav_dop` | `los_enu` (≥ 4 unit vectors) | (HᵀH)⁻¹-derived GDOP / PDOP / HDOP / VDOP / TDOP. |
| `nav_klobuchar` | `gps_tow_s`, `lat_deg`, `lon_deg`, `elevation_deg`, `azimuth_deg`, `alpha`, `beta` | Single-frequency Klobuchar ionospheric delay (m). |
| `nav_saastamoinen` | `height_m`, `elevation_deg`, `pressure_hpa?`, `temp_k?`, `e_w_hpa?` | Saastamoinen zenith-tropospheric delay, mapped to slant. |
| `nav_ecef_to_enu` | `ref_lat`, `ref_lon`, `ref_alt_m`, `x`, `y`, `z` | Transform an ECEF point into the local east-north-up frame. |
| `nav_imu_drift` | `gyro_random_walk_deg_sqrt_hr`, `bias_instability_deg_per_hr`, `scale_factor_ppm`, `time_s`, `rate_deg_s?` | Composite IMU attitude error vs time. |

## Example uses

- **Geometry sanity.** Build a synthetic four-satellite constellation,
  push the LOS unit vectors through `nav_dop` — confirm GDOP ≈ √(PDOP² +
  TDOP²).
- **Single-frequency receiver error budget.** `nav_klobuchar` for iono
  + `nav_saastamoinen` for tropo = a rough total range-bias.
- **Coast time.** `nav_imu_drift` for an aviation-grade IMU at 60 s →
  the attitude error you carry while GPS is unavailable.

## Notes

- `nav_dop` expects vectors in **ENU**. If your LOS is in ECEF, project
  via `nav_ecef_to_enu` first.
- `nav_imu_drift` is an RSS approximation — adequate for spec sheets,
  not for certification work.

## See also

- [tools.md](../tools.md)
- [skills/geodesy.md](geodesy.md) — ECEF ↔ geodetic + UTM / MGRS.
- [skills/satellite.md](satellite.md) — SGP4 orbit propagation.
