# Earth models — `earth_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/earth_models.rs`](../../src/skills/earth_models.rs) |
| **Tools** | `earth_sidereal_time`, `earth_magnetic_declination` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |

## What it does

Two earth-model helpers that don't fit anywhere else: mean sidereal time
(for radio-astronomy / equatorial-coordinate work) and a centred-dipole
magnetic-declination estimate (for compass corrections).

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `earth_sidereal_time` | `when?` (RFC 3339, defaults to now), `longitude_deg?` | Greenwich (and optionally local) mean sidereal time in hours, via Meeus formula 12.4. |
| `earth_magnetic_declination` | `lat_deg`, `lon_deg`, `year?` | Magnetic declination in degrees (east positive), centred-dipole approximation (2025 epoch with 0.07°/year drift). |

## Example uses

- **Telescope pointing.** Pair `earth_sidereal_time` with an
  observer-frame right ascension to compute the hour angle.
- **Compass correction.** `earth_magnetic_declination { lat_deg: 40.7,
  lon_deg: -74.0 }` — apply to a magnetic bearing to get true.

## Notes

- **Approximations only.** A real navigation-grade compass correction
  needs the full WMM2020 coefficient set (not bundled); peak error of
  this dipole model is ~1°.
- **EGM2008 geoid undulation** and full tidal harmonic constants are
  larger data files and are explicitly out of scope for v0.1.2.

## See also

- [tools.md](../tools.md)
- [skills/astro.md](astro.md) — sun / moon position, rise/transit/set.
- [skills/geodesy.md](geodesy.md) — ECEF / WGS84 transforms.
