# Mesh & 3-D interchange — `interchange_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/interchange.rs`](../../src/skills/interchange.rs) |
| **Tools** | `interchange_stl_info` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |
| **Dep** | `base64` (for binary STL via base64) |

## What it does

Read-only metadata probe for 3-D mesh interchange formats. Today it
covers STL (binary + ASCII). The eventual goal is a small portfolio of
interchange-format probes (MAVLink / NetCDF / DICOM / OBJ / PLY); STL
is shipped first because it's the cleanest entry point.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `interchange_stl_info` | `data_base64?` **or** `data_ascii?` | Probe an STL mesh. Returns triangle count, axis-aligned bounding box (`bbox_min`, `bbox_max`), total surface area, and centroid. Auto-detects binary vs ASCII from the input you pass. |

## Example uses

- **3-D printing sanity check.** Hand a base64-encoded binary STL —
  inspect `triangle_count` and `bbox` to confirm scale before sending
  to a slicer.
- **CAD-pipeline metadata.** ASCII STL pulled from a file path → quick
  centroid for visual annotation.

## Notes

- The tool **does not** validate manifoldness or watertightness — that
  needs a geometry kernel.
- For files on disk, base64-encode them at the call site (the
  filesystem family stays gated separately).

## See also

- [tools.md](../tools.md)
- [skills/new_charts.md](new_charts.md) — `chart_density_map` for
  visualizing point clouds derived from vertex centroids.
