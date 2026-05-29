# Geometry — `geo_distance`, `geo_azimuth`, `geometry_formula`, `geometry_formula_list`

|  |  |
| --- | --- |
| **Module** | [`src/skills/geometry.rs`](../../src/skills/geometry.rs) |
| **Tools** | `geo_distance`, `geo_azimuth`, `geometry_formula`, `geometry_formula_list` |
| **Network** | none (local) |
| **Default** | on; gateable via `[tools]` |

## What it does
Great-circle (haversine) distance and bearing between coordinates, plus named
geometry formulas (areas, volumes, distances, Pythagoras, Heron, law of cosines) via
the shared formula registry. Angles in degrees.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `geo_distance` | `lat1`, `lon1`, `lat2`, `lon2` | Great-circle distance (km + mi). |
| `geo_azimuth` | `lat1`, `lon1`, `lat2`, `lon2` | Initial bearing + back azimuth, with compass labels. |
| `geometry_formula` | `name`, `args` | Compute, e.g. `circle_area`, `sphere_volume`, `pythagorean`, `heron_area`, `law_of_cosines_side`. |
| `geometry_formula_list` | `filter?` | Discover the geometry formula ids. |

## Example uses
- `geometry_formula { name: "sphere_volume", args: { r: 2 } }`.
- `geo_distance { lat1: 51.5, lon1: -0.13, lat2: 48.86, lon2: 2.35 }` (London→Paris).

## See also
[tools.md](../tools.md) · [trigonometry.md](trigonometry.md)
