# Geospatial format converters — `convert_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/geo_convert.rs`](../../src/skills/geo_convert.rs) |
| **Tools** | `convert_nmea_decode`, `convert_cot_encode`, `convert_geojson_to_wkt` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |

## What it does

Three small format converters for geospatial inter-op — decode a
NMEA-0183 GPS sentence into typed fields, emit a Cursor-on-Target XML
event for TAK pipelines, and convert a GeoJSON geometry to WKT.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `convert_nmea_decode` | `sentence` | Parse `$GPGGA / GPRMC / GPGSA / GPGSV / GPVTG`; verifies the XOR checksum. Returns the sentence type and structured fields. |
| `convert_cot_encode` | `uid`, `cot_type`, `lat`, `lon`, `hae_m?`, `stale_seconds?`, `callsign?` | Emit a CoT XML event (TAK-ingestible) — `<event>` with `<point>` and optional `<detail><contact callsign>`. |
| `convert_geojson_to_wkt` | `geojson` | Convert a GeoJSON Geometry (or a Feature's geometry) to Well-Known Text. Supports Point, LineString, Polygon, MultiPoint, MultiLineString, MultiPolygon. |

## Example uses

- **NMEA → JSON.** Pipe `$GPGGA,123519,4807.038,N,…,*47` through
  `convert_nmea_decode` and use the returned `lat` / `lon` /
  `altitude_m` downstream.
- **Push a contact onto a TAK server.** `convert_cot_encode { uid: "u-1",
  cot_type: "a-f-G-U-C", lat, lon, callsign: "ALPHA-7" }` → XML you can
  POST to a TAK ingress.
- **Spatial DB load.** `convert_geojson_to_wkt` → `INSERT INTO … VALUES
  (ST_GeomFromText('POLYGON …', 4326))`.

## Notes

- Sentence parsing is permissive about field absence — empty strings
  decode to `null` / 0 where appropriate.
- The CoT emit uses the standard "m-g" `how` and a default `ce` /
  `le` of 9 999 999 m; tighten if you have real accuracy.

## See also

- [tools.md](../tools.md)
- [skills/geodesy.md](geodesy.md) — Vincenty / UTM / MGRS / ECEF
  underpin the coordinate work.
