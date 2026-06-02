# Geodesy & coordinate systems — `geo_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/geodesy.rs`](../../src/skills/geodesy.rs) |
| **Tools** | `geo_vincenty_inverse`, `geo_vincenty_direct`, `geo_great_circle_polyline`, `geo_cross_track`, `geo_polygon_area_geodesic`, `geo_utm_from_latlon`, `geo_latlon_from_utm`, `geo_mgrs_from_latlon`, `geo_latlon_from_mgrs`, `geo_ecef_from_latlon`, `geo_latlon_from_ecef`, `geo_helmert` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |
| **Dep** | `geographiclib-rs` (Karney) |
| **Datum** | WGS84 — `a = 6 378 137 m`, `f = 1/298.257223563` |

## What it does

A full ellipsoidal toolkit on the WGS84 datum: geodesic distance / bearing
(Vincenty inverse and direct), great-circle polyline densify, cross-track
distance, ellipsoidal polygon area, UTM and MGRS forward/inverse, ECEF ↔
geodetic, and a 7-parameter Helmert datum transform.

For two-point great-circle distance + bearing that doesn't need
sub-millimetre precision, the older spherical-earth tools
[`geo_distance` / `geo_azimuth`](geometry.md) in the `geometry` family
remain available.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `geo_vincenty_inverse` | `lat1`, `lon1`, `lat2`, `lon2` | Distance (m) + initial & final azimuths between two coordinates. |
| `geo_vincenty_direct` | `lat`, `lon`, `azimuth_deg`, `distance_m` | Destination + final azimuth after travelling along a geodesic. |
| `geo_great_circle_polyline` | `lat1`, `lon1`, `lat2`, `lon2`, `n` | `n` equally-spaced points along the geodesic (inclusive of endpoints). |
| `geo_cross_track` | `lat`, `lon`, `lat1`, `lon1`, `lat2`, `lon2` | Cross-track distance (m) from a point to a great-circle path. |
| `geo_polygon_area_geodesic` | `vertices` | Signed ellipsoidal area (m²) + perimeter (m) of a polygon. |
| `geo_utm_from_latlon` | `lat`, `lon` | UTM zone, hemisphere, easting, northing. |
| `geo_latlon_from_utm` | `zone`, `hemisphere`, `easting`, `northing` | Inverse of the above. |
| `geo_mgrs_from_latlon` | `lat`, `lon`, `precision?` | MGRS string; precision in digits per axis (1..5). |
| `geo_latlon_from_mgrs` | `mgrs` | Inverse of the above. |
| `geo_ecef_from_latlon` | `lat`, `lon`, `alt_m?` | Geodetic (with ellipsoidal height) → ECEF (m). |
| `geo_latlon_from_ecef` | `x`, `y`, `z` | ECEF → geodetic via Bowring's iterative method. |
| `geo_helmert` | `x`, `y`, `z`, `tx`, `ty`, `tz`, `rx_arcsec`, `ry_arcsec`, `rz_arcsec`, `scale_ppm` | 7-parameter (position-vector convention) datum shift. |

## Example uses

- **Range / bearing.** "What's the geodesic between JFK and LHR?" →
  `geo_vincenty_inverse` returns ~5 552 km and the initial azimuth.
- **Polyline for plotting.** Densify a route with `geo_great_circle_polyline`,
  then feed the points into `chart_density_map` or
  [`convert_geojson_to_wkt`](geo_convert.md).
- **Compatibility.** Translate a NAD27 ECEF triple to WGS84 with `geo_helmert`
  using the published transformation parameters; then `geo_latlon_from_ecef`.

## Notes

- **Altitudes** are ellipsoidal (relative to WGS84), **not** mean sea level.
  Apply a geoid offset (not bundled) when you need orthometric height.
- **MGRS precision** maps to ground resolution: `5` = 1 m, `4` = 10 m,
  `3` = 100 m, `2` = 1 km, `1` = 10 km.

## See also

- [tools.md](../tools.md)
- [skills/nav_aiding.md](nav_aiding.md) — ECEF → ENU + DOP for receiver work.
- [skills/geo_convert.md](geo_convert.md) — emit GeoJSON / WKT / NMEA / CoT.
- [skills/geometry.md](geometry.md) — simple spherical-earth distance.
