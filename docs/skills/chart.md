# Charts & plots — `chart_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/chart.rs`](../../src/skills/chart.rs) |
| **Tools** | `chart_line`, `chart_bar`, `chart_scatter`, `chart_histogram`, `chart_pie`, `chart_heatmap`, `chart_grafana`, `chart_stat`, `chart_gauge`, `chart_bar_gauge`, `chart_state_timeline`, `chart_candlestick`, `chart_sparkline`, `chart_canvas`, `chart_interactive`, `chart_mermaid` |
| **Network** | none — pure-Rust SVG generation (CDN load is client-side for `chart_interactive` only) |
| **Default** | **on** — gated by `[chart]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[chart].enabled` via `LODESTONE_CHART_ENABLED`. Defaults live in [`src/config.rs`](../../src/config.rs). |

## What it does

Sixteen chart tools, all generating self-contained output the model can hand
back to its client. **Static charts are SVG with a `viewBox`**, so they scale
to the renderer's viewport — responsive layout without JavaScript. SVG is
delivered as MCP `image/svg+xml` content (clients that render images show it
inline) plus a one-line text fallback (clients that don't get a description).

Three groups:

- **matplotlib equivalents** — `chart_line`, `chart_bar`, `chart_scatter`,
  `chart_histogram`, `chart_pie`. Multi-series line gets a tab10 palette + a
  legend; the histogram auto-bins to √n when `bins` is omitted; the scatter
  takes an optional point size. Both `chart_line` and `chart_scatter` accept
  ISO-8601 date strings as `x` values (`"2026-01-15"`, `"2026-01-15T12:34:56Z"`,
  …) — strings are auto-parsed to Unix timestamps and ticks rendered as dates.
- **Grafana operational panels** — `chart_grafana` (dark time series),
  `chart_stat` (big-number tile + sparkline), `chart_gauge` (270° radial dial),
  `chart_bar_gauge` (Top-N tile), `chart_state_timeline` (categorical bands),
  `chart_candlestick` (OHLC), `chart_sparkline` (tiny inline trend). For when
  "this is operational telemetry" needs to read at a glance.
- **Procedural / escape hatches** — `chart_heatmap` (matrix + colorbar,
  colormaps viridis / magma / plasma / coolwarm / grayscale), `chart_canvas`
  (turtle / Logo / matplotlib.patches procedural drawing — `line` / `rect` /
  `circle` / `polygon` / `polyline` / `text` commands), `chart_interactive`
  (wraps Chart.js or Plotly; emits self-contained HTML that loads the library
  from a CDN — clients that render HTML get full interactivity), and
  `chart_mermaid` (wraps user-supplied mermaid source in a markdown code fence
  — every modern MCP client renders mermaid blocks natively).

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `chart_line` | `series`, `title?`, `xlabel?`, `ylabel?`, `width?`, `height?` | Multi-series line plot. `x` values can be numbers OR ISO-8601 strings. |
| `chart_bar` | `labels`, `values`, `title?`, … | Vertical bar chart. |
| `chart_scatter` | `points`, `title?`, `point_size?`, … | Scatter plot. `x` values can be numbers or ISO-8601 strings. |
| `chart_histogram` | `values`, `bins?`, `title?`, … | Histogram, auto-bins to √n when omitted. |
| `chart_pie` | `slices`, `title?`, … | Pie chart with percentage legend. |
| `chart_heatmap` | `matrix`, `row_labels?`, `col_labels?`, `colormap?`, … | 2D matrix as colored cells with a colorbar. Colormaps: `viridis` (default), `magma`, `plasma`, `coolwarm`, `grayscale`. |
| `chart_grafana` | `title?`, `series`, `unit?`, … | Dark-themed time-series panel with translucent area fills + last-value labels. |
| `chart_stat` | `value`, `label?`, `unit?`, `thresholds?`, `sparkline?`, `color_mode?` | Big-number tile, threshold-tinted, optional background sparkline. `color_mode="background"` flood-fills the tile. |
| `chart_gauge` | `value`, `min`, `max`, `thresholds?`, `unit?`, `title?` | 270° radial dial with threshold bands. |
| `chart_bar_gauge` | `items`, `min`, `max`, `thresholds?`, `unit?` | One horizontal threshold-tinted bar per item — Top-N tile. |
| `chart_state_timeline` | `rows`, `state_colors?` | Categorical state bands per row. Sensible defaults: `up=green`, `degraded=yellow`, `down=red`, `scheduled=blue`, `unknown=gray`. Override via `state_colors`. |
| `chart_candlestick` | `candles`, `up_color?`, `down_color?`, … | OHLC bodies + wicks. Financial / market time-series. |
| `chart_sparkline` | `points`, `color?`, `fill_opacity?`, … | Tiny inline trend with no chrome. The shape Edward Tufte popularized. |
| `chart_canvas` | `commands`, `width?`, `height?`, `background?`, `title?` | Procedural drawing: `line` / `rect` / `circle` / `polygon` / `polyline` / `text` commands drawn in order. |
| `chart_interactive` | `library` (`chartjs` or `plotly`), `config`, `title?`, `width?`, `height?` | Self-contained HTML wrapping Chart.js or Plotly. Clients that render HTML get full interactivity (hover, zoom, pan, legend toggling, responsive resize); others see source. |
| `chart_mermaid` | `source`, `title?` | Wrap mermaid source in a markdown code fence. |

## Example uses

- **Time series with date axis** —
  `chart_line { series: [{ name: "AAPL close", points: [["2026-01-02", 185.6], ["2026-01-03", 186.1], …] }] }`.
- **Operational dashboard** —
  `chart_stat { value: 99.4, label: "API uptime", unit: "%", thresholds: [{ at: 99.9, color: "green" }, { at: 99, color: "yellow" }, { at: 0, color: "red" }], sparkline: [99.6, 99.4, 99.3, 99.4] }`.
- **Categorical state grid** —
  `chart_state_timeline { rows: [{ name: "auth-svc", states: [{ at: "2026-05-30T00:00:00Z", state: "up" }, { at: "2026-05-30T02:14:00Z", state: "degraded" }] }] }`.
- **Heatmap** —
  `chart_heatmap { matrix: [[…], …], colormap: "coolwarm" }`.
- **Mermaid passthrough** —
  `chart_mermaid { source: "flowchart LR\\n  a-->b\\n  b-->c" }`.
- **Interactive Plotly** —
  `chart_interactive { library: "plotly", config: { data: [{ x: [...], y: [...], type: "scatter" }], layout: { … } } }`.

## Notes

- **Verify HTML output before shipping.** Pipe `chart_interactive`'s HTML
  through [`html_render`](html.md) to catch JS errors, missing CDN loads, or
  config typos.
- **Tab10 palette** is the default for multi-series — same as matplotlib's
  default cycle.
- **Date strings on `x` axes** work for `chart_line`, `chart_scatter`, and
  `chart_grafana`. Numeric and string `x` can mix within one series but you
  probably don't want them to.

## See also

- [tools.md](../tools.md)
- [skills/html.md](html.md) — verify `chart_interactive` output runs cleanly.
- [skills/yahoo.md](yahoo.md) / [skills/stocks.md](stocks.md) — common upstream
  data for `chart_candlestick`.
