# Satellite tracking — `sat_tle` / `sat_position` / `sat_observe`

|  |  |
| --- | --- |
| **Module** | [`src/skills/satellite.rs`](../../src/skills/satellite.rs) |
| **Tools** | `sat_tle`, `sat_position`, `sat_observe` |
| **Network** | local compute (SGP4); `sat_tle` fetches a TLE from CelesTrak (keyless) |
| **Default** | always on (no gate; individual tools gateable via `[tools]`) |
| **Config** | no tunables; noted in [`config/17-data-apis.toml`](../../config/17-data-apis.toml) |

## What it does
Propagates a Two-Line Element set (TLE) with SGP4 and reports either the satellite's
ground sub-point or its look-angles from an observer — all **local compute**. The
only network call is `sat_tle`, which fetches a satellite's *current* TLE from
CelesTrak (keyless). The intended workflow is `sat_tle` → `sat_position` /
`sat_observe`. Internally SGP4 yields a TEME position rotated to ECEF by GMST, then
converted to WGS-84 geodetic; observer look-angles use the topocentric SEZ frame.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `sat_tle` | `query` | Fetch the current TLE from CelesTrak by NORAD catalog number (e.g. `25544` = ISS) or name (e.g. `ISS`). Returns the name + two TLE lines. |
| `sat_position` | `tle_line1`, `tle_line2`, `at?` | SGP4 sub-point at a time: latitude, longitude, altitude (km), orbital speed (km/s). `at` is RFC3339 (also accepts `YYYY-MM-DD HH:MM:SS` / `YYYY-MM-DD`); omit for now. |
| `sat_observe` | `tle_line1`, `tle_line2`, `observer_lat`, `observer_lon`, `observer_alt_km?`, `at?` | Look-angles from an observer: azimuth°, elevation° (negative = below horizon), slant range (km). `observer_alt_km` defaults to 0; `at` as above. |

## Configuration & gating
- No config section and no gate — the family is always available (see the note in
  [`config/17-data-apis.toml`](../../config/17-data-apis.toml)). Individual tools are
  gateable via `[tools]`.
- All three are read-only / pure-compute (no confirmation guard). `sat_tle` results
  are cached.

## Example uses
- **Is the ISS overhead now?** — `sat_tle "ISS"` to get the current TLE, then
  `sat_observe` with those two lines plus your `observer_lat`/`observer_lon`; a
  positive elevation means it's above your horizon.
- **Track an upcoming pass** — `sat_tle 25544` → call `sat_observe` repeatedly with
  different `at` times to find when elevation goes positive.
- **Where is it right now?** — `sat_tle "HUBBLE"` → `sat_position` for the current
  lat/lon/altitude sub-point.

## See also
[tools.md](../tools.md)
