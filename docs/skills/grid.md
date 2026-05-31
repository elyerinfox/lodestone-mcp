# Power grid & critical infrastructure — `grid_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/grid.rs`](../../src/skills/grid.rs) |
| **Tools** | `grid_power_plants`, `grid_transmission_lines`, `grid_substations`, `grid_data_centres`, `grid_gas_pipelines`, `grid_submarine_cables` |
| **Network** | OpenStreetMap Overpass — **keyless** |
| **Default** | **on** |
| **Config** | none |

## What it does

Thin typed wrappers over `osm_overpass` that preformulate the QL for
critical-infrastructure layers — the same layers OpenGridWorks visualizes
(power plants, transmission lines, substations, data centres, gas pipelines,
submarine cables). The model picks a bounding box; the tool runs the right
query.

For arbitrary OSM tag queries, use [`osm_overpass`](osm.md) directly.

## Tools

All take a `(south, west, north, east)` bounding box plus an optional `max`
(default 100, capped at 1000).

| Tool | Purpose |
| --- | --- |
| `grid_power_plants` | OSM `power=plant` features in the bbox. |
| `grid_transmission_lines` | OSM `power=line` ways. |
| `grid_substations` | OSM `power=substation`. |
| `grid_data_centres` | OSM `telecom=data_center`. |
| `grid_gas_pipelines` | OSM `man_made=pipeline` + substance filter. |
| `grid_submarine_cables` | OSM `submarine=yes` (telecom + power). |

## Example uses

- **Power plants in Washington State** —
  `grid_power_plants { south: 45.5, west: -124.9, north: 49.0, east: -116.9 }`.
- **Submarine cables landing on the US west coast** —
  `grid_submarine_cables { south: 32.0, west: -125.0, north: 49.0, east: -117.0 }`.

## Notes

- **Overpass User-Agent.** The Apache instance at `overpass-api.de` returns
  406 for browser-like UAs and `curl/*`; the shared `lodestone-mcp/…` UA
  satisfies the OSM policy.
- **Sparsely tagged areas.** OSM coverage is uneven — a remote bbox may
  return less than reality contains.
- **Cached.** Results pass through the retrieval cache.

## See also

- [tools.md](../tools.md)
- [skills/osm.md](osm.md) — arbitrary Overpass queries.
