# Specialist chart types — `chart_polar`, `chart_smith`, `chart_waterfall`, `chart_compass_rose`, `chart_skyplot`, `chart_density_map`

|  |  |
| --- | --- |
| **Module** | [`src/skills/new_charts.rs`](../../src/skills/new_charts.rs) |
| **Tools** | `chart_polar`, `chart_smith`, `chart_waterfall`, `chart_compass_rose`, `chart_skyplot`, `chart_density_map` |
| **Network** | none — local compute |
| **Default** | on (rides with `[chart]`) |

## What it does

Six SVG generators following the existing [`chart`](chart.md) family
pattern — antenna patterns, RF impedance, spectrogram waterfalls, wind
roses, satellite az/el dome views, and 2-D density heatmaps. Each
returns an `image/svg+xml` content piece plus a one-line text
description, just like the chart family.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `chart_polar` | `magnitudes`, `angles_deg?`, `use_db?`, `db_min?`, `title?` | Polar magnitude-vs-angle plot. Default is dB (peak normalized to 0 dB). |
| `chart_smith` | `impedances`, `z0?`, `labels?`, `title?` | Smith chart — normalized impedances on the Γ-plane with constant-R and constant-X arcs. |
| `chart_waterfall` | `power`, `db_min?`, `db_max?`, `freq_label?`, `title?` | Spectrogram waterfall heatmap (time × frequency × power) with viridis colormap + colorbar. |
| `chart_compass_rose` | `magnitudes_by_bearing`, `title?` | Compass / wind rose with cardinal markers; one bar per bearing slice (16 typical). |
| `chart_skyplot` | `az_el`, `labels?`, `title?` | Sky plot — zenith at center, horizon (el=0) at the outer ring; one labelled marker per object. |
| `chart_density_map` | `points`, `nx?`, `ny?`, `title?` | 2-D density heatmap binned from raw `[x, y]` samples (default 32 × 32). |

## Example uses

- **Antenna pattern.** Pass 360 magnitudes (one per degree) to
  `chart_polar` — the dB grid + main lobe + sidelobes draw themselves.
- **Impedance match.** `chart_smith { impedances: [[50, 0], [25, 25]],
  z0: 50, labels: ["matched", "shunted"] }` — quick visual of where
  the load sits.
- **STFT waterfall.** Feed `signal_spectrogram`'s power matrix straight
  into `chart_waterfall`.
- **Wind direction frequency.** Bin a year of weather data into 16
  bearing slices, `chart_compass_rose`.
- **Visible satellites.** Take SGP4 output, build `az_el`, → `chart_skyplot`.

## Notes

- All charts are SVG and scale to the renderer's viewport (no fixed
  pixel size). Clients that don't render images get a one-line text
  fallback describing the chart.
- The Smith chart's reactance arcs are clipped to the outer Γ-disc via
  `clip-path: circle(...)` — clients that don't honour that attribute
  will show arcs extending past the disc.

## See also

- [tools.md](../tools.md)
- [skills/chart.md](chart.md) — line / bar / scatter / heatmap / etc.
- [skills/dsp_advanced.md](dsp_advanced.md) — `signal_spectrogram` feeds
  `chart_waterfall`.
- [skills/satellite.md](satellite.md) — SGP4 → az/el → `chart_skyplot`.
