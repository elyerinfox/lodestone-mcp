# OpenStreetMap & GIS — `osm_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/osm.rs`](../../src/skills/osm.rs) |
| **Tools** | `osm_geocode`, `osm_reverse_geocode`, `osm_overpass`, `osm_elevation`, `osm_route` |
| **Network** | Nominatim / Overpass / Open-Elevation / OSRM public demo — **keyless** |
| **Default** | **on** |
| **Config** | none — like wikipedia / arxiv |

## What it does

OpenStreetMap-ecosystem tools: place-name ↔ lat/lon (**Nominatim**),
arbitrary feature queries (**Overpass**), elevation lookups
(**Open-Elevation**), and routing (**OSRM public demo**). All keyless, all
cached.

For distance / bearing between two coordinates, the existing `geo_distance` /
`geo_azimuth` tools in the [geometry](geometry.md) skill are local-only and
faster.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `osm_geocode` | `query`, `max_results?` | Place name → lat/lon via Nominatim. |
| `osm_reverse_geocode` | `lat`, `lon` | Lat/lon → nearest address. |
| `osm_overpass` | `query`, `max_elements?` | Run an arbitrary Overpass-QL query. |
| `osm_elevation` | `points` | Open-Elevation: ground elevation (m AMSL) for up to 100 lat/lon pairs. |
| `osm_route` | `from_lat`, `from_lon`, `to_lat`, `to_lon`, `profile?` | OSRM driving / walking / cycling route summary. |

## Example uses

- **Where is this?** —
  `osm_geocode { query: "Space Needle, Seattle" }`.
- **What's around me?** —
  `osm_overpass { query: "[out:json][timeout:25];node[amenity=cafe](around:200,47.62,-122.35);out;" }`.
- **Elevation at a peak** —
  `osm_elevation { points: [[46.852, -121.760]] }` (Rainier summit ≈ 4392 m).
- **Quick route** —
  `osm_route { from_lat: 47.6, from_lon: -122.3, to_lat: 47.7, to_lon: -122.1 }`.

## Notes

- **Keyless usage policies.** Nominatim and Overpass ask for a descriptive
  User-Agent (the project's `lodestone-mcp/…` UA) and reasonable rate. OSRM's
  public demo is best-effort; for production routing, self-host.
- **Cached.** Results pass through the retrieval cache.

## See also

- [tools.md](../tools.md)
- [skills/geometry.md](geometry.md) — local `geo_distance` / `geo_azimuth`.
- [skills/grid.md](grid.md) — preformulated Overpass queries for
  critical-infrastructure layers.
