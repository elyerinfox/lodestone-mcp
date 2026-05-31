//! Chart / plot rendering. Pure-Rust SVG generator, no external deps, no
//! network. Each tool emits SVG markup wrapped as MCP `image/svg+xml`
//! content — a compliant client renders the chart inline; a text fallback
//! describes the figure in one line for clients that don't render images.
//!
//! Five plot tools plus a mermaid-source helper:
//!
//! * `chart_line` — multi-series line plot.
//! * `chart_bar` — vertical bar chart, single series.
//! * `chart_scatter` — scatter plot from `(x, y)` points.
//! * `chart_histogram` — histogram from raw values with auto- or
//!   user-supplied bin count.
//! * `chart_pie` — pie chart from labeled slices.
//! * `chart_mermaid` — wraps mermaid source in a markdown code fence with a
//!   rendering hint; no server-side rasterization needed because every modern
//!   MCP client (Claude Code, LM Studio, Cursor) renders ```mermaid blocks.
//!
//! All charts are SVG with a `viewBox`, so they scale to the renderer's
//! viewport rather than to a fixed pixel size — "responsive" in the layout
//! sense without needing JavaScript.

use std::fmt::Write as _;
use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, Content, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

pub const TOOL_NAMES: &[&str] = &[
    "chart_line",
    "chart_bar",
    "chart_scatter",
    "chart_histogram",
    "chart_pie",
    "chart_heatmap",
    "chart_canvas",
    "chart_grafana",
    "chart_interactive",
    "chart_mermaid",
];

// ---------------------------------------------------------------------------
// SVG engine — small, dependency-free, self-contained
// ---------------------------------------------------------------------------

/// Default plot width / height in user-space units. The `<svg>` tag emits a
/// `viewBox` matching these dimensions so the actual rendered size is
/// driven by the viewing client.
const DEFAULT_W: f64 = 760.0;
const DEFAULT_H: f64 = 460.0;

/// Inset (in user-space units) reserved for axis labels / titles around the
/// data plot area.
struct Margins {
    top: f64,
    right: f64,
    bottom: f64,
    left: f64,
}

impl Default for Margins {
    fn default() -> Self {
        Self {
            top: 36.0,
            right: 24.0,
            bottom: 56.0,
            left: 64.0,
        }
    }
}

/// Tab10-style colour palette — high contrast, dataviz-safe.
const PALETTE: &[&str] = &[
    "#1f77b4", "#ff7f0e", "#2ca02c", "#d62728", "#9467bd", "#8c564b", "#e377c2", "#7f7f7f",
    "#bcbd22", "#17becf",
];

/// Compute "nice" tick values across `[min, max]`. Targets roughly `n_target`
/// labels by rounding the step to {1, 2, 2.5, 5} × 10^k. Produces ticks that
/// include the endpoints (or the next nice step beyond).
fn nice_ticks(mut min: f64, mut max: f64, n_target: usize) -> Vec<f64> {
    if !min.is_finite() || !max.is_finite() {
        return vec![0.0, 1.0];
    }
    if (max - min).abs() < f64::EPSILON {
        // Single value: produce a degenerate but well-formed pair so the
        // axis still renders.
        let center = min;
        let pad = if center.abs() > 0.0 {
            center.abs()
        } else {
            1.0
        };
        min = center - pad;
        max = center + pad;
    }
    let range = max - min;
    let raw_step = range / (n_target.max(1) as f64);
    let mag = 10f64.powf(raw_step.abs().log10().floor());
    let frac = raw_step.abs() / mag;
    let nice = if frac <= 1.0 {
        1.0
    } else if frac <= 2.0 {
        2.0
    } else if frac <= 2.5 {
        2.5
    } else if frac <= 5.0 {
        5.0
    } else {
        10.0
    };
    let step = nice * mag * raw_step.signum().abs();
    let nmin = (min / step).floor() * step;
    let nmax = (max / step).ceil() * step;
    let mut out = Vec::new();
    let mut v = nmin;
    while v <= nmax + step * 1e-9 {
        out.push(round_to_step(v, step));
        v += step;
    }
    out
}

fn round_to_step(v: f64, step: f64) -> f64 {
    // Avoid floating-point ugliness in tick labels (e.g. 0.30000000000000004).
    let decimals = if step >= 1.0 {
        0
    } else {
        (-step.abs().log10().floor()).max(0.0) as i32
    };
    let m = 10f64.powi(decimals);
    (v * m).round() / m
}

/// Format a tick value compactly. Drops trailing zeros after the decimal
/// point unless that makes the value lose precision; uses scientific
/// notation only when the magnitude warrants it.
fn fmt_tick(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let av = v.abs();
    if !(1e-3..1e6).contains(&av) {
        return format!("{v:.2e}");
    }
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// Escape `<`, `>`, `&`, `"`, `'` for safe embedding inside SVG text /
/// attribute values. We control the markup but not the user-supplied
/// titles / labels.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// `<svg>` document open tag with a `viewBox` so the chart scales to the
/// rendering viewport. The `font-family` cascade falls back through common
/// sans-serif stacks so no external font is required.
fn svg_open(w: f64, h: f64) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" \
         preserveAspectRatio=\"xMidYMid meet\" \
         style=\"font-family:ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif;background:#ffffff\" \
         role=\"img\">",
    )
}

/// Render axis tick lines + labels for the X (bottom) axis. `domain` is
/// data-space min/max; `range` is the pixel-space left/right edge of the
/// plot area.
fn render_x_axis(
    out: &mut String,
    ticks: &[f64],
    domain: (f64, f64),
    range: (f64, f64),
    baseline_y: f64,
    plot_top: f64,
) {
    let (dmin, dmax) = domain;
    let (rmin, rmax) = range;
    let scale = |v: f64| -> f64 {
        if (dmax - dmin).abs() < f64::EPSILON {
            (rmin + rmax) / 2.0
        } else {
            rmin + (v - dmin) / (dmax - dmin) * (rmax - rmin)
        }
    };
    let _ = writeln!(
        out,
        "<line x1=\"{rmin}\" y1=\"{baseline_y}\" x2=\"{rmax}\" y2=\"{baseline_y}\" \
         stroke=\"#999\" stroke-width=\"1\"/>"
    );
    for t in ticks {
        let x = scale(*t);
        let _ = writeln!(
            out,
            "<line x1=\"{x}\" y1=\"{plot_top}\" x2=\"{x}\" y2=\"{baseline_y}\" \
             stroke=\"#eee\" stroke-width=\"1\"/>\n\
             <line x1=\"{x}\" y1=\"{baseline_y}\" x2=\"{x}\" y2=\"{tick_top}\" stroke=\"#999\" stroke-width=\"1\"/>\n\
             <text x=\"{x}\" y=\"{label_y}\" text-anchor=\"middle\" font-size=\"11\" fill=\"#444\">{lbl}</text>",
            tick_top = baseline_y + 4.0,
            label_y = baseline_y + 16.0,
            lbl = esc(&fmt_tick(*t)),
        );
    }
}

/// Y axis. Mirror of `render_x_axis`. `domain` is bottom→top; `range`
/// pixel-space is top→bottom (smaller pixel y = higher value).
fn render_y_axis(
    out: &mut String,
    ticks: &[f64],
    domain: (f64, f64),
    range: (f64, f64),
    axis_x: f64,
    plot_right: f64,
) {
    let (dmin, dmax) = domain;
    let (rmin, rmax) = range; // rmin = top pixel, rmax = bottom pixel
    let scale = |v: f64| -> f64 {
        if (dmax - dmin).abs() < f64::EPSILON {
            (rmin + rmax) / 2.0
        } else {
            rmax - (v - dmin) / (dmax - dmin) * (rmax - rmin)
        }
    };
    let _ = writeln!(
        out,
        "<line x1=\"{axis_x}\" y1=\"{rmin}\" x2=\"{axis_x}\" y2=\"{rmax}\" stroke=\"#999\" stroke-width=\"1\"/>"
    );
    for t in ticks {
        let y = scale(*t);
        let _ = writeln!(
            out,
            "<line x1=\"{axis_x}\" y1=\"{y}\" x2=\"{plot_right}\" y2=\"{y}\" stroke=\"#eee\" stroke-width=\"1\"/>\n\
             <line x1=\"{tick_left}\" y1=\"{y}\" x2=\"{axis_x}\" y2=\"{y}\" stroke=\"#999\" stroke-width=\"1\"/>\n\
             <text x=\"{label_x}\" y=\"{y_text}\" text-anchor=\"end\" font-size=\"11\" fill=\"#444\">{lbl}</text>",
            tick_left = axis_x - 4.0,
            label_x = axis_x - 8.0,
            y_text = y + 4.0,
            lbl = esc(&fmt_tick(*t)),
        );
    }
}

/// Compute (xmin, xmax, ymin, ymax) over a list of `(x, y)` series.
/// Returns padded ranges suitable for `nice_ticks` and a plot area.
fn auto_xy_range(series: &[Vec<(f64, f64)>]) -> (f64, f64, f64, f64) {
    let mut xmin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    for s in series {
        for &(x, y) in s {
            if x.is_finite() {
                xmin = xmin.min(x);
                xmax = xmax.max(x);
            }
            if y.is_finite() {
                ymin = ymin.min(y);
                ymax = ymax.max(y);
            }
        }
    }
    if !xmin.is_finite() {
        xmin = 0.0;
        xmax = 1.0;
    }
    if !ymin.is_finite() {
        ymin = 0.0;
        ymax = 1.0;
    }
    (xmin, xmax, ymin, ymax)
}

/// Wrap an SVG figure title + axis labels around the plot area.
fn render_chrome(
    out: &mut String,
    title: Option<&str>,
    xlabel: Option<&str>,
    ylabel: Option<&str>,
    w: f64,
    h: f64,
    m: &Margins,
) {
    if let Some(t) = title {
        let _ = writeln!(
            out,
            "<text x=\"{cx}\" y=\"22\" text-anchor=\"middle\" font-size=\"15\" fill=\"#111\" font-weight=\"600\">{lbl}</text>",
            cx = w / 2.0,
            lbl = esc(t),
        );
    }
    if let Some(x) = xlabel {
        let _ = writeln!(
            out,
            "<text x=\"{cx}\" y=\"{ly}\" text-anchor=\"middle\" font-size=\"12\" fill=\"#444\">{lbl}</text>",
            cx = (m.left + (w - m.right)) / 2.0,
            ly = h - 14.0,
            lbl = esc(x),
        );
    }
    if let Some(y) = ylabel {
        let _ = writeln!(
            out,
            "<text x=\"16\" y=\"{cy}\" text-anchor=\"middle\" font-size=\"12\" fill=\"#444\" \
             transform=\"rotate(-90 16 {cy})\">{lbl}</text>",
            cy = (m.top + (h - m.bottom)) / 2.0,
            lbl = esc(y),
        );
    }
}

/// Inline legend drawn at the top-right of the plot area. Skipped when only
/// one series is shown.
fn render_legend(
    out: &mut String,
    labels: &[&str],
    colors: &[&str],
    plot_right: f64,
    plot_top: f64,
) {
    if labels.len() < 2 {
        return;
    }
    let mut y = plot_top + 4.0;
    for (i, lbl) in labels.iter().enumerate() {
        let c = colors[i % colors.len()];
        let _ = writeln!(
            out,
            "<rect x=\"{x}\" y=\"{y}\" width=\"12\" height=\"12\" fill=\"{c}\"/>\n\
             <text x=\"{tx}\" y=\"{ty}\" font-size=\"11\" fill=\"#222\">{txt}</text>",
            x = plot_right - 160.0,
            tx = plot_right - 144.0,
            ty = y + 10.0,
            txt = esc(lbl),
        );
        y += 16.0;
    }
}

// ---------------------------------------------------------------------------
// Common — MCP result construction
// ---------------------------------------------------------------------------

/// Base64-encode without an external crate. ~30 lines, standard alphabet,
/// padding included. Used to embed SVG bytes inside MCP `image/svg+xml`
/// content.
fn b64(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | (input[i + 2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push_str("==");
    } else if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

/// Wrap an SVG string as MCP image content + a one-line text description.
/// Clients that render images show the chart inline; clients that don't get
/// a descriptive line.
fn svg_result(svg: String, description: String) -> CallToolResult {
    let img = Content::image(b64(svg.as_bytes()), "image/svg+xml");
    let txt = Content::text(description);
    CallToolResult::success(vec![img, txt])
}

// ---------------------------------------------------------------------------
// chart_line
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LineSeries {
    /// Label shown in the legend.
    label: String,
    /// `(x, y)` data points. Must contain at least 2 points.
    points: Vec<[f64; 2]>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LineArgs {
    /// One or more named series to plot together on the same axes.
    series: Vec<LineSeries>,
    /// Title above the plot.
    #[serde(default)]
    title: Option<String>,
    /// X-axis label.
    #[serde(default)]
    xlabel: Option<String>,
    /// Y-axis label.
    #[serde(default)]
    ylabel: Option<String>,
    /// Plot width in user-space units (the SVG viewBox; renders scale up/down
    /// to fit the viewport). Default 760, capped at 4000.
    #[serde(default)]
    width: Option<f64>,
    /// Plot height. Default 460, capped at 4000.
    #[serde(default)]
    height: Option<f64>,
}

pub struct ChartLine;
impl Skill for ChartLine {
    fn name(&self) -> &'static str {
        "chart_line"
    }
    fn description(&self) -> &'static str {
        "Render one or more (x, y) series as a line plot. Returns an SVG with a viewBox so the \
        figure scales to the client's viewport. Each `series` is `{label, points: [[x, y], ...]}`. \
        Optional `title`, `xlabel`, `ylabel`, `width`, `height`. Multi-series gets a legend, \
        tab10 palette."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<LineArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<LineArgs>()?;
            if args.series.is_empty() {
                return Err(invalid(
                    "`series` must contain at least one entry".to_string(),
                ));
            }
            let series_xy: Vec<Vec<(f64, f64)>> = args
                .series
                .iter()
                .map(|s| s.points.iter().map(|p| (p[0], p[1])).collect())
                .collect();
            for (i, s) in series_xy.iter().enumerate() {
                if s.len() < 2 {
                    return Err(invalid(format!(
                        "series {} (\"{}\") needs at least 2 points",
                        i, args.series[i].label
                    )));
                }
            }
            let w = args.width.unwrap_or(DEFAULT_W).clamp(160.0, 4000.0);
            let h = args.height.unwrap_or(DEFAULT_H).clamp(120.0, 4000.0);
            let m = Margins::default();
            let plot_left = m.left;
            let plot_right = w - m.right;
            let plot_top = m.top;
            let plot_bottom = h - m.bottom;
            let (xmin, xmax, ymin, ymax) = auto_xy_range(&series_xy);
            let x_ticks = nice_ticks(xmin, xmax, 7);
            let y_ticks = nice_ticks(ymin, ymax, 6);
            let xd = (
                *x_ticks.first().unwrap_or(&xmin),
                *x_ticks.last().unwrap_or(&xmax),
            );
            let yd = (
                *y_ticks.first().unwrap_or(&ymin),
                *y_ticks.last().unwrap_or(&ymax),
            );
            let mut svg = svg_open(w, h);
            render_chrome(
                &mut svg,
                args.title.as_deref(),
                args.xlabel.as_deref(),
                args.ylabel.as_deref(),
                w,
                h,
                &m,
            );
            render_x_axis(
                &mut svg,
                &x_ticks,
                xd,
                (plot_left, plot_right),
                plot_bottom,
                plot_top,
            );
            render_y_axis(
                &mut svg,
                &y_ticks,
                yd,
                (plot_top, plot_bottom),
                plot_left,
                plot_right,
            );
            let scale_x = |x: f64| {
                plot_left + (x - xd.0) / (xd.1 - xd.0).max(f64::EPSILON) * (plot_right - plot_left)
            };
            let scale_y = |y: f64| {
                plot_bottom
                    - (y - yd.0) / (yd.1 - yd.0).max(f64::EPSILON) * (plot_bottom - plot_top)
            };
            for (i, s) in series_xy.iter().enumerate() {
                let color = PALETTE[i % PALETTE.len()];
                let mut path = String::new();
                for (j, (x, y)) in s.iter().enumerate() {
                    let cmd = if j == 0 { 'M' } else { 'L' };
                    let _ = write!(path, "{cmd}{:.2},{:.2} ", scale_x(*x), scale_y(*y));
                }
                let _ = writeln!(
                    svg,
                    "<path d=\"{path}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\" \
                     stroke-linejoin=\"round\" stroke-linecap=\"round\"/>"
                );
            }
            let labels: Vec<&str> = args.series.iter().map(|s| s.label.as_str()).collect();
            render_legend(&mut svg, &labels, PALETTE, plot_right, plot_top);
            svg.push_str("</svg>");
            let total_points: usize = series_xy.iter().map(|s| s.len()).sum();
            let desc = format!(
                "Line chart{} · {} serie{} · {} points · x ∈ [{}, {}] · y ∈ [{}, {}]",
                args.title
                    .as_deref()
                    .map(|t| format!(" \"{}\"", t))
                    .unwrap_or_default(),
                args.series.len(),
                if args.series.len() == 1 { "" } else { "s" },
                total_points,
                fmt_tick(xmin),
                fmt_tick(xmax),
                fmt_tick(ymin),
                fmt_tick(ymax),
            );
            Ok(svg_result(svg, desc))
        })
    }
}

// ---------------------------------------------------------------------------
// chart_bar
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BarArgs {
    /// One label per bar (category names on the X axis).
    labels: Vec<String>,
    /// Bar heights, parallel to `labels`.
    values: Vec<f64>,
    /// Title above the plot.
    #[serde(default)]
    title: Option<String>,
    /// X-axis label.
    #[serde(default)]
    xlabel: Option<String>,
    /// Y-axis label.
    #[serde(default)]
    ylabel: Option<String>,
    /// Plot width in user-space units. Default 760.
    #[serde(default)]
    width: Option<f64>,
    /// Plot height. Default 460.
    #[serde(default)]
    height: Option<f64>,
}

pub struct ChartBar;
impl Skill for ChartBar {
    fn name(&self) -> &'static str {
        "chart_bar"
    }
    fn description(&self) -> &'static str {
        "Render a single-series vertical bar chart. `labels` and `values` must be the same length; \
        bars are drawn left-to-right in input order. Returns SVG with a viewBox so the figure \
        scales to the client viewport."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BarArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<BarArgs>()?;
            if args.labels.is_empty() {
                return Err(invalid(
                    "`labels` must contain at least one entry".to_string(),
                ));
            }
            if args.labels.len() != args.values.len() {
                return Err(invalid(
                    "`labels` and `values` must be the same length".to_string(),
                ));
            }
            let w = args.width.unwrap_or(DEFAULT_W).clamp(160.0, 4000.0);
            let h = args.height.unwrap_or(DEFAULT_H).clamp(120.0, 4000.0);
            let m = Margins::default();
            let plot_left = m.left;
            let plot_right = w - m.right;
            let plot_top = m.top;
            let plot_bottom = h - m.bottom;
            let max_v = args
                .values
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            let min_v = args.values.iter().copied().fold(f64::INFINITY, f64::min);
            let (ymin, ymax) = (min_v.min(0.0), max_v.max(0.0));
            let y_ticks = nice_ticks(ymin, ymax, 6);
            let yd = (
                *y_ticks.first().unwrap_or(&ymin),
                *y_ticks.last().unwrap_or(&ymax),
            );
            let mut svg = svg_open(w, h);
            render_chrome(
                &mut svg,
                args.title.as_deref(),
                args.xlabel.as_deref(),
                args.ylabel.as_deref(),
                w,
                h,
                &m,
            );
            render_y_axis(
                &mut svg,
                &y_ticks,
                yd,
                (plot_top, plot_bottom),
                plot_left,
                plot_right,
            );
            let n = args.labels.len() as f64;
            let band = (plot_right - plot_left) / n;
            let bar_w = band * 0.7;
            let baseline_y = if yd.0 <= 0.0 && yd.1 >= 0.0 {
                // zero baseline maps to its actual position
                plot_bottom
                    - (0.0 - yd.0) / (yd.1 - yd.0).max(f64::EPSILON) * (plot_bottom - plot_top)
            } else {
                plot_bottom
            };
            let _ = writeln!(
                svg,
                "<line x1=\"{plot_left}\" y1=\"{baseline_y}\" x2=\"{plot_right}\" \
                 y2=\"{baseline_y}\" stroke=\"#999\" stroke-width=\"1\"/>"
            );
            for (i, (lbl, v)) in args.labels.iter().zip(args.values.iter()).enumerate() {
                let cx = plot_left + (i as f64 + 0.5) * band;
                let y_v = plot_bottom
                    - (*v - yd.0) / (yd.1 - yd.0).max(f64::EPSILON) * (plot_bottom - plot_top);
                let (top, height) = if *v >= 0.0 {
                    (y_v, baseline_y - y_v)
                } else {
                    (baseline_y, y_v - baseline_y)
                };
                let _ = writeln!(
                    svg,
                    "<rect x=\"{x}\" y=\"{top}\" width=\"{bar_w}\" height=\"{height}\" \
                     fill=\"{c}\" opacity=\"0.85\"/>\n\
                     <text x=\"{cx}\" y=\"{ly}\" text-anchor=\"middle\" font-size=\"11\" \
                     fill=\"#444\">{txt}</text>",
                    x = cx - bar_w / 2.0,
                    c = PALETTE[0],
                    ly = plot_bottom + 16.0,
                    txt = esc(lbl),
                );
            }
            svg.push_str("</svg>");
            let total: f64 = args.values.iter().sum();
            let desc = format!(
                "Bar chart{} · {} bars · sum {} · min {} · max {}",
                args.title
                    .as_deref()
                    .map(|t| format!(" \"{}\"", t))
                    .unwrap_or_default(),
                args.values.len(),
                fmt_tick(total),
                fmt_tick(min_v),
                fmt_tick(max_v),
            );
            Ok(svg_result(svg, desc))
        })
    }
}

// ---------------------------------------------------------------------------
// chart_scatter
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ScatterArgs {
    /// `(x, y)` points.
    points: Vec<[f64; 2]>,
    /// Title above the plot.
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    xlabel: Option<String>,
    #[serde(default)]
    ylabel: Option<String>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
    /// Marker radius in user-space units. Default 4.
    #[serde(default)]
    point_size: Option<f64>,
}

pub struct ChartScatter;
impl Skill for ChartScatter {
    fn name(&self) -> &'static str {
        "chart_scatter"
    }
    fn description(&self) -> &'static str {
        "Render `(x, y)` points as a scatter plot. Useful for showing data distributions / \
        correlations without committing to an interpolation between samples. Returns SVG with a \
        viewBox."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ScatterArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ScatterArgs>()?;
            if args.points.is_empty() {
                return Err(invalid(
                    "`points` must contain at least one entry".to_string(),
                ));
            }
            let w = args.width.unwrap_or(DEFAULT_W).clamp(160.0, 4000.0);
            let h = args.height.unwrap_or(DEFAULT_H).clamp(120.0, 4000.0);
            let r = args.point_size.unwrap_or(4.0).clamp(1.0, 20.0);
            let m = Margins::default();
            let plot_left = m.left;
            let plot_right = w - m.right;
            let plot_top = m.top;
            let plot_bottom = h - m.bottom;
            let series: Vec<Vec<(f64, f64)>> =
                vec![args.points.iter().map(|p| (p[0], p[1])).collect()];
            let (xmin, xmax, ymin, ymax) = auto_xy_range(&series);
            let x_ticks = nice_ticks(xmin, xmax, 7);
            let y_ticks = nice_ticks(ymin, ymax, 6);
            let xd = (
                *x_ticks.first().unwrap_or(&xmin),
                *x_ticks.last().unwrap_or(&xmax),
            );
            let yd = (
                *y_ticks.first().unwrap_or(&ymin),
                *y_ticks.last().unwrap_or(&ymax),
            );
            let mut svg = svg_open(w, h);
            render_chrome(
                &mut svg,
                args.title.as_deref(),
                args.xlabel.as_deref(),
                args.ylabel.as_deref(),
                w,
                h,
                &m,
            );
            render_x_axis(
                &mut svg,
                &x_ticks,
                xd,
                (plot_left, plot_right),
                plot_bottom,
                plot_top,
            );
            render_y_axis(
                &mut svg,
                &y_ticks,
                yd,
                (plot_top, plot_bottom),
                plot_left,
                plot_right,
            );
            let scale_x = |x: f64| {
                plot_left + (x - xd.0) / (xd.1 - xd.0).max(f64::EPSILON) * (plot_right - plot_left)
            };
            let scale_y = |y: f64| {
                plot_bottom
                    - (y - yd.0) / (yd.1 - yd.0).max(f64::EPSILON) * (plot_bottom - plot_top)
            };
            for [x, y] in &args.points {
                let _ = writeln!(
                    svg,
                    "<circle cx=\"{cx:.2}\" cy=\"{cy:.2}\" r=\"{r}\" fill=\"{c}\" \
                     opacity=\"0.7\"/>",
                    cx = scale_x(*x),
                    cy = scale_y(*y),
                    c = PALETTE[0],
                );
            }
            svg.push_str("</svg>");
            let desc = format!(
                "Scatter{} · {} points · x ∈ [{}, {}] · y ∈ [{}, {}]",
                args.title
                    .as_deref()
                    .map(|t| format!(" \"{}\"", t))
                    .unwrap_or_default(),
                args.points.len(),
                fmt_tick(xmin),
                fmt_tick(xmax),
                fmt_tick(ymin),
                fmt_tick(ymax),
            );
            Ok(svg_result(svg, desc))
        })
    }
}

// ---------------------------------------------------------------------------
// chart_histogram
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HistogramArgs {
    /// Raw observations to bin.
    values: Vec<f64>,
    /// Number of equal-width bins. Defaults to Freedman-Diaconis-style
    /// √n heuristic, clamped to [5, 60].
    #[serde(default)]
    bins: Option<u32>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    xlabel: Option<String>,
    #[serde(default)]
    ylabel: Option<String>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
}

pub struct ChartHistogram;
impl Skill for ChartHistogram {
    fn name(&self) -> &'static str {
        "chart_histogram"
    }
    fn description(&self) -> &'static str {
        "Bin a sequence of raw values into equal-width buckets and render as bars. `bins` is \
        optional — defaults to √n (clamped to [5, 60]). Returns SVG."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HistogramArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<HistogramArgs>()?;
            if args.values.is_empty() {
                return Err(invalid(
                    "`values` must contain at least one observation".to_string(),
                ));
            }
            let vmin = args.values.iter().copied().fold(f64::INFINITY, f64::min);
            let vmax = args
                .values
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            if !vmin.is_finite() || !vmax.is_finite() {
                return Err(invalid("values must be finite".to_string()));
            }
            let bins = args
                .bins
                .map(|b| b as usize)
                .unwrap_or_else(|| (args.values.len() as f64).sqrt().ceil() as usize)
                .clamp(5, 60);
            let width = (vmax - vmin).max(f64::EPSILON) / bins as f64;
            let mut counts = vec![0_u64; bins];
            for v in &args.values {
                let idx = ((v - vmin) / width).floor() as i64;
                let idx = idx.clamp(0, bins as i64 - 1) as usize;
                counts[idx] += 1;
            }
            let w = args.width.unwrap_or(DEFAULT_W).clamp(160.0, 4000.0);
            let h = args.height.unwrap_or(DEFAULT_H).clamp(120.0, 4000.0);
            let m = Margins::default();
            let plot_left = m.left;
            let plot_right = w - m.right;
            let plot_top = m.top;
            let plot_bottom = h - m.bottom;
            let max_count = *counts.iter().max().unwrap_or(&1) as f64;
            let y_ticks = nice_ticks(0.0, max_count, 6);
            let yd = (
                *y_ticks.first().unwrap_or(&0.0),
                *y_ticks.last().unwrap_or(&max_count),
            );
            let x_ticks = nice_ticks(vmin, vmax, 7);
            let xd = (
                *x_ticks.first().unwrap_or(&vmin),
                *x_ticks.last().unwrap_or(&vmax),
            );
            let mut svg = svg_open(w, h);
            render_chrome(
                &mut svg,
                args.title.as_deref(),
                args.xlabel.as_deref(),
                args.ylabel.as_deref(),
                w,
                h,
                &m,
            );
            render_x_axis(
                &mut svg,
                &x_ticks,
                xd,
                (plot_left, plot_right),
                plot_bottom,
                plot_top,
            );
            render_y_axis(
                &mut svg,
                &y_ticks,
                yd,
                (plot_top, plot_bottom),
                plot_left,
                plot_right,
            );
            let scale_x = |x: f64| {
                plot_left + (x - xd.0) / (xd.1 - xd.0).max(f64::EPSILON) * (plot_right - plot_left)
            };
            let bar_top_y = |count: u64| {
                plot_bottom
                    - (count as f64 - yd.0) / (yd.1 - yd.0).max(f64::EPSILON)
                        * (plot_bottom - plot_top)
            };
            for (i, c) in counts.iter().enumerate() {
                let lo = vmin + i as f64 * width;
                let hi = lo + width;
                let x_l = scale_x(lo);
                let x_h = scale_x(hi);
                let y_t = bar_top_y(*c);
                let bw = (x_h - x_l - 1.0).max(0.5);
                let _ = writeln!(
                    svg,
                    "<rect x=\"{x_l:.2}\" y=\"{y_t:.2}\" width=\"{bw:.2}\" height=\"{bh:.2}\" \
                     fill=\"{col}\" opacity=\"0.85\"/>",
                    bh = plot_bottom - y_t,
                    col = PALETTE[0],
                );
            }
            svg.push_str("</svg>");
            let mean = args.values.iter().sum::<f64>() / args.values.len() as f64;
            let desc = format!(
                "Histogram{} · n = {} · {} bins · range [{}, {}] · mean {}",
                args.title
                    .as_deref()
                    .map(|t| format!(" \"{}\"", t))
                    .unwrap_or_default(),
                args.values.len(),
                bins,
                fmt_tick(vmin),
                fmt_tick(vmax),
                fmt_tick(mean),
            );
            Ok(svg_result(svg, desc))
        })
    }
}

// ---------------------------------------------------------------------------
// chart_pie
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PieSlice {
    label: String,
    value: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PieArgs {
    /// Labeled slices. Each slice's value must be ≥ 0.
    slices: Vec<PieSlice>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
}

pub struct ChartPie;
impl Skill for ChartPie {
    fn name(&self) -> &'static str {
        "chart_pie"
    }
    fn description(&self) -> &'static str {
        "Render labeled slices as a pie chart with a legend. Each `slice` is `{label, value}`. \
        Slice angles are proportional to `value / sum(values)`. Returns SVG."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PieArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<PieArgs>()?;
            if args.slices.is_empty() {
                return Err(invalid(
                    "`slices` must contain at least one entry".to_string(),
                ));
            }
            let total: f64 = args.slices.iter().map(|s| s.value.max(0.0)).sum();
            if total <= 0.0 {
                return Err(invalid(
                    "slice values sum to 0; nothing to draw".to_string(),
                ));
            }
            let w = args.width.unwrap_or(DEFAULT_W).clamp(160.0, 4000.0);
            let h = args.height.unwrap_or(DEFAULT_H).clamp(120.0, 4000.0);
            let r = ((w.min(h)) * 0.36).max(20.0);
            let cx = w * 0.36;
            let cy = h * 0.55;
            let mut svg = svg_open(w, h);
            if let Some(t) = args.title.as_deref() {
                let _ = writeln!(
                    svg,
                    "<text x=\"{x}\" y=\"22\" text-anchor=\"middle\" font-size=\"15\" \
                     fill=\"#111\" font-weight=\"600\">{lbl}</text>",
                    x = w / 2.0,
                    lbl = esc(t),
                );
            }
            let mut angle_acc: f64 = -std::f64::consts::FRAC_PI_2; // start at 12 o'clock
            for (i, s) in args.slices.iter().enumerate() {
                let frac = (s.value.max(0.0)) / total;
                let theta = frac * std::f64::consts::TAU;
                let a0 = angle_acc;
                let a1 = angle_acc + theta;
                let x0 = cx + r * a0.cos();
                let y0 = cy + r * a0.sin();
                let x1 = cx + r * a1.cos();
                let y1 = cy + r * a1.sin();
                let large = if theta > std::f64::consts::PI { 1 } else { 0 };
                let color = PALETTE[i % PALETTE.len()];
                let _ = writeln!(
                    svg,
                    "<path d=\"M{cx:.2},{cy:.2} L{x0:.2},{y0:.2} \
                     A{r:.2},{r:.2} 0 {large} 1 {x1:.2},{y1:.2} Z\" \
                     fill=\"{color}\" opacity=\"0.85\" stroke=\"#fff\" stroke-width=\"1\"/>"
                );
                angle_acc = a1;
            }
            // Legend down the right side.
            let lx = w * 0.72;
            let mut ly = h * 0.20;
            for (i, s) in args.slices.iter().enumerate() {
                let color = PALETTE[i % PALETTE.len()];
                let pct = (s.value.max(0.0)) / total * 100.0;
                let _ = writeln!(
                    svg,
                    "<rect x=\"{lx}\" y=\"{ly}\" width=\"12\" height=\"12\" fill=\"{color}\"/>\n\
                     <text x=\"{tx}\" y=\"{ty}\" font-size=\"12\" fill=\"#222\">{lbl} · {pct:.1}%</text>",
                    tx = lx + 16.0,
                    ty = ly + 10.0,
                    lbl = esc(&s.label),
                );
                ly += 18.0;
            }
            svg.push_str("</svg>");
            let desc = format!(
                "Pie chart{} · {} slices · total {}",
                args.title
                    .as_deref()
                    .map(|t| format!(" \"{}\"", t))
                    .unwrap_or_default(),
                args.slices.len(),
                fmt_tick(total),
            );
            Ok(svg_result(svg, desc))
        })
    }
}

// ---------------------------------------------------------------------------
// chart_mermaid — diagrams-as-code (no server-side rasterization)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MermaidArgs {
    /// Mermaid source (no leading code fence — we'll wrap it). Supports every
    /// mermaid diagram type: flowchart, sequenceDiagram, classDiagram,
    /// stateDiagram, erDiagram, gantt, gitGraph, mindmap, journey, pie,
    /// timeline, etc.
    source: String,
    /// Optional short caption emitted above the rendered block.
    #[serde(default)]
    title: Option<String>,
}

pub struct ChartMermaid;
impl Skill for ChartMermaid {
    fn name(&self) -> &'static str {
        "chart_mermaid"
    }
    fn description(&self) -> &'static str {
        "Wrap user-supplied Mermaid source in a markdown ```mermaid``` block with an optional \
        caption. Every modern MCP client (Claude Code, LM Studio, Cursor, GitHub, etc.) renders \
        mermaid code fences natively — no server-side rasterization is needed, and the result \
        scales / re-themes with the client. For built-in plot types (line / bar / scatter / \
        histogram / pie), call the corresponding `chart_*` tool instead and get an SVG image \
        directly."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MermaidArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<MermaidArgs>()?;
            let src = args.source.trim();
            if src.is_empty() {
                return Err(invalid("`source` must not be empty".to_string()));
            }
            let mut out = String::new();
            if let Some(t) = args
                .title
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let _ = writeln!(out, "**{}**", t);
                out.push('\n');
            }
            out.push_str("```mermaid\n");
            out.push_str(src);
            if !src.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("```\n");
            Ok(text_result(out))
        })
    }
}

// ---------------------------------------------------------------------------
// chart_heatmap — 2D matrix as colored cells (science / ML staple)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HeatmapArgs {
    /// 2D matrix of values. `matrix[row][col]` — outer length is the number
    /// of rows, inner length is the number of columns. All rows must have
    /// the same length.
    matrix: Vec<Vec<f64>>,
    /// Optional row labels (one per row).
    #[serde(default)]
    row_labels: Option<Vec<String>>,
    /// Optional column labels (one per column).
    #[serde(default)]
    col_labels: Option<Vec<String>>,
    /// Colormap. Accepts: `"viridis"` (default — perceptually uniform),
    /// `"magma"`, `"plasma"`, `"coolwarm"` (diverging, good for signed
    /// values), `"grayscale"`.
    #[serde(default)]
    colormap: Option<String>,
    /// Title above the plot.
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
}

/// 5-stop colormap interpolator. The lookup tables are coarse but sufficient
/// for 8-bit display; we lerp between adjacent stops.
fn colormap_stops(name: &str) -> &'static [[u8; 3]] {
    match name {
        "magma" => &[
            [0, 0, 4],
            [80, 18, 123],
            [182, 54, 121],
            [252, 137, 97],
            [252, 253, 191],
        ],
        "plasma" => &[
            [13, 8, 135],
            [126, 3, 168],
            [203, 71, 119],
            [248, 149, 64],
            [240, 249, 33],
        ],
        "coolwarm" => &[
            [59, 76, 192],
            [144, 178, 254],
            [221, 221, 221],
            [245, 156, 125],
            [180, 4, 38],
        ],
        "grayscale" | "gray" | "grey" => &[
            [0, 0, 0],
            [64, 64, 64],
            [128, 128, 128],
            [192, 192, 192],
            [255, 255, 255],
        ],
        // default: viridis
        _ => &[
            [68, 1, 84],
            [59, 82, 139],
            [33, 145, 140],
            [94, 201, 98],
            [253, 231, 37],
        ],
    }
}

fn colormap_lookup(stops: &[[u8; 3]], t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let n = stops.len() - 1;
    let x = t * n as f64;
    let i = x.floor() as usize;
    let f = x - i as f64;
    let i = i.min(n);
    let j = (i + 1).min(n);
    [
        (stops[i][0] as f64 + (stops[j][0] as f64 - stops[i][0] as f64) * f) as u8,
        (stops[i][1] as f64 + (stops[j][1] as f64 - stops[i][1] as f64) * f) as u8,
        (stops[i][2] as f64 + (stops[j][2] as f64 - stops[i][2] as f64) * f) as u8,
    ]
}

pub struct ChartHeatmap;
impl Skill for ChartHeatmap {
    fn name(&self) -> &'static str {
        "chart_heatmap"
    }
    fn description(&self) -> &'static str {
        "Render a 2D matrix as a colored heatmap. Cell color is mapped through one of \
        viridis (default), magma, plasma, coolwarm, or grayscale. Optional `row_labels` and \
        `col_labels`. Common for covariance / correlation / confusion matrices, attention maps, \
        image intensities, etc."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HeatmapArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<HeatmapArgs>()?;
            if args.matrix.is_empty() {
                return Err(invalid(
                    "`matrix` must contain at least one row".to_string(),
                ));
            }
            let ncols = args.matrix[0].len();
            if ncols == 0 {
                return Err(invalid("rows must be non-empty".to_string()));
            }
            for r in &args.matrix {
                if r.len() != ncols {
                    return Err(invalid("all rows must be the same length".to_string()));
                }
            }
            let nrows = args.matrix.len();
            let mut vmin = f64::INFINITY;
            let mut vmax = f64::NEG_INFINITY;
            for r in &args.matrix {
                for &v in r {
                    if v.is_finite() {
                        vmin = vmin.min(v);
                        vmax = vmax.max(v);
                    }
                }
            }
            if !vmin.is_finite() {
                vmin = 0.0;
                vmax = 1.0;
            }
            if (vmax - vmin).abs() < f64::EPSILON {
                vmax = vmin + 1.0;
            }
            let w = args.width.unwrap_or(720.0).clamp(160.0, 4000.0);
            let h = args.height.unwrap_or(520.0).clamp(120.0, 4000.0);
            let m = Margins {
                top: 36.0,
                right: 96.0, // reserve room for colorbar
                bottom: 64.0,
                left: 80.0,
            };
            let plot_left = m.left;
            let plot_right = w - m.right;
            let plot_top = m.top;
            let plot_bottom = h - m.bottom;
            let cell_w = (plot_right - plot_left) / ncols as f64;
            let cell_h = (plot_bottom - plot_top) / nrows as f64;
            let stops = colormap_stops(args.colormap.as_deref().unwrap_or("viridis"));
            let mut svg = svg_open(w, h);
            if let Some(t) = args.title.as_deref() {
                let _ = writeln!(
                    svg,
                    "<text x=\"{cx}\" y=\"22\" text-anchor=\"middle\" font-size=\"15\" \
                     fill=\"#111\" font-weight=\"600\">{lbl}</text>",
                    cx = w / 2.0,
                    lbl = esc(t),
                );
            }
            for (ri, row) in args.matrix.iter().enumerate() {
                for (ci, &v) in row.iter().enumerate() {
                    let t = (v - vmin) / (vmax - vmin);
                    let [r, g, b] = colormap_lookup(stops, t);
                    let x = plot_left + ci as f64 * cell_w;
                    let y = plot_top + ri as f64 * cell_h;
                    let _ = writeln!(
                        svg,
                        "<rect x=\"{x:.2}\" y=\"{y:.2}\" width=\"{cell_w:.2}\" \
                         height=\"{cell_h:.2}\" fill=\"rgb({r},{g},{b})\"/>"
                    );
                }
            }
            // Row labels (left, vertically centered per cell).
            if let Some(labels) = args.row_labels.as_ref() {
                for (i, lbl) in labels.iter().enumerate().take(nrows) {
                    let y = plot_top + (i as f64 + 0.5) * cell_h + 4.0;
                    let _ = writeln!(
                        svg,
                        "<text x=\"{x}\" y=\"{y}\" text-anchor=\"end\" font-size=\"11\" \
                         fill=\"#333\">{lbl}</text>",
                        x = plot_left - 6.0,
                        lbl = esc(lbl),
                    );
                }
            }
            // Column labels (rotated -45° below cells).
            if let Some(labels) = args.col_labels.as_ref() {
                for (i, lbl) in labels.iter().enumerate().take(ncols) {
                    let cx = plot_left + (i as f64 + 0.5) * cell_w;
                    let cy = plot_bottom + 18.0;
                    let _ = writeln!(
                        svg,
                        "<text x=\"{cx}\" y=\"{cy}\" font-size=\"11\" fill=\"#333\" \
                         transform=\"rotate(-45 {cx} {cy})\" text-anchor=\"end\">{lbl}</text>",
                        lbl = esc(lbl),
                    );
                }
            }
            // Colorbar on the right.
            let cb_x = plot_right + 12.0;
            let cb_w = 14.0;
            let cb_top = plot_top;
            let cb_bot = plot_bottom;
            let steps = 64;
            for i in 0..steps {
                let t0 = i as f64 / steps as f64;
                let [r, g, b] = colormap_lookup(stops, 1.0 - t0);
                let y0 = cb_top + t0 * (cb_bot - cb_top);
                let y1 = cb_top + ((i + 1) as f64 / steps as f64) * (cb_bot - cb_top);
                let _ = writeln!(
                    svg,
                    "<rect x=\"{cb_x:.2}\" y=\"{y0:.2}\" width=\"{cb_w:.2}\" \
                     height=\"{h:.2}\" fill=\"rgb({r},{g},{b})\"/>",
                    h = y1 - y0
                );
            }
            // Colorbar tick labels (vmin / mid / vmax).
            let mid = (vmin + vmax) / 2.0;
            for (frac, val) in [(0.0_f64, vmax), (0.5, mid), (1.0, vmin)] {
                let y = cb_top + frac * (cb_bot - cb_top) + 4.0;
                let _ = writeln!(
                    svg,
                    "<text x=\"{x}\" y=\"{y}\" font-size=\"10\" fill=\"#333\">{lbl}</text>",
                    x = cb_x + cb_w + 4.0,
                    lbl = esc(&fmt_tick(val)),
                );
            }
            svg.push_str("</svg>");
            let desc = format!(
                "Heatmap{} · {}×{} matrix · range [{}, {}]",
                args.title
                    .as_deref()
                    .map(|t| format!(" \"{}\"", t))
                    .unwrap_or_default(),
                nrows,
                ncols,
                fmt_tick(vmin),
                fmt_tick(vmax),
            );
            Ok(svg_result(svg, desc))
        })
    }
}

// ---------------------------------------------------------------------------
// chart_canvas — procedural drawing (turtle / Logo / svg primitives)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind")]
enum CanvasCommand {
    /// Straight line between two points.
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        #[serde(default)]
        stroke: Option<String>,
        #[serde(default)]
        width: Option<f64>,
    },
    /// Axis-aligned rectangle.
    Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        #[serde(default)]
        fill: Option<String>,
        #[serde(default)]
        stroke: Option<String>,
    },
    /// Circle at center, radius `r`.
    Circle {
        cx: f64,
        cy: f64,
        r: f64,
        #[serde(default)]
        fill: Option<String>,
        #[serde(default)]
        stroke: Option<String>,
    },
    /// Closed polygon over a list of points.
    Polygon {
        points: Vec<[f64; 2]>,
        #[serde(default)]
        fill: Option<String>,
        #[serde(default)]
        stroke: Option<String>,
    },
    /// Open polyline.
    Polyline {
        points: Vec<[f64; 2]>,
        #[serde(default)]
        stroke: Option<String>,
        #[serde(default)]
        width: Option<f64>,
    },
    /// Text label at a position.
    Text {
        x: f64,
        y: f64,
        text: String,
        #[serde(default)]
        fill: Option<String>,
        #[serde(default)]
        size: Option<f64>,
        #[serde(default)]
        anchor: Option<String>, // start / middle / end
    },
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CanvasArgs {
    /// Drawing commands executed in order. Coordinates are in user-space
    /// units (0,0 = top-left; positive y is down — SVG convention). Use
    /// `width` / `height` to set the viewBox.
    commands: Vec<CanvasCommand>,
    /// ViewBox width in user-space units. Default 800.
    #[serde(default)]
    width: Option<f64>,
    /// ViewBox height in user-space units. Default 600.
    #[serde(default)]
    height: Option<f64>,
    /// Background fill (any SVG color, e.g. "#fff" or "white" or "transparent").
    /// Default "#ffffff".
    #[serde(default)]
    background: Option<String>,
    /// Optional title rendered above the canvas.
    #[serde(default)]
    title: Option<String>,
}

fn color_or(opt: &Option<String>, default: &str) -> String {
    opt.as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(default)
        .to_string()
}

pub struct ChartCanvas;
impl Skill for ChartCanvas {
    fn name(&self) -> &'static str {
        "chart_canvas"
    }
    fn description(&self) -> &'static str {
        "Procedural drawing surface (turtle / Logo / matplotlib.patches style). Issue a sequence \
        of low-level primitives — line, rect, circle, polygon, polyline, text — and the tool emits \
        a self-contained SVG. Use for custom diagrams, illustrations, board layouts, anything that \
        doesn't fit a standard chart type. Commands take `kind` plus their geometry; colors are \
        any valid SVG color (named, hex, rgb())."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CanvasArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<CanvasArgs>()?;
            if args.commands.is_empty() {
                return Err(invalid(
                    "`commands` must contain at least one entry".to_string(),
                ));
            }
            let w = args.width.unwrap_or(800.0).clamp(16.0, 8000.0);
            let h = args.height.unwrap_or(600.0).clamp(16.0, 8000.0);
            let bg = color_or(&args.background, "#ffffff");
            let mut svg = svg_open(w, h);
            let _ = writeln!(
                svg,
                "<rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" fill=\"{}\"/>",
                esc(&bg)
            );
            if let Some(t) = args.title.as_deref() {
                let _ = writeln!(
                    svg,
                    "<text x=\"{cx}\" y=\"22\" text-anchor=\"middle\" font-size=\"15\" \
                     fill=\"#111\" font-weight=\"600\">{lbl}</text>",
                    cx = w / 2.0,
                    lbl = esc(t),
                );
            }
            for cmd in &args.commands {
                match cmd {
                    CanvasCommand::Line {
                        x1,
                        y1,
                        x2,
                        y2,
                        stroke,
                        width,
                    } => {
                        let _ = writeln!(
                            svg,
                            "<line x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" \
                             stroke=\"{s}\" stroke-width=\"{wid}\" stroke-linecap=\"round\"/>",
                            s = esc(&color_or(stroke, "#222")),
                            wid = width.unwrap_or(1.5),
                        );
                    }
                    CanvasCommand::Rect {
                        x,
                        y,
                        width,
                        height,
                        fill,
                        stroke,
                    } => {
                        let _ = writeln!(
                            svg,
                            "<rect x=\"{x}\" y=\"{y}\" width=\"{width}\" height=\"{height}\" \
                             fill=\"{f}\" stroke=\"{s}\" stroke-width=\"1\"/>",
                            f = esc(&color_or(fill, "transparent")),
                            s = esc(&color_or(stroke, "#222")),
                        );
                    }
                    CanvasCommand::Circle {
                        cx,
                        cy,
                        r,
                        fill,
                        stroke,
                    } => {
                        let _ = writeln!(
                            svg,
                            "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" fill=\"{f}\" \
                             stroke=\"{s}\" stroke-width=\"1\"/>",
                            f = esc(&color_or(fill, "transparent")),
                            s = esc(&color_or(stroke, "#222")),
                        );
                    }
                    CanvasCommand::Polygon {
                        points,
                        fill,
                        stroke,
                    } => {
                        if points.is_empty() {
                            continue;
                        }
                        let pts: String = points
                            .iter()
                            .map(|p| format!("{},{}", p[0], p[1]))
                            .collect::<Vec<_>>()
                            .join(" ");
                        let _ = writeln!(
                            svg,
                            "<polygon points=\"{pts}\" fill=\"{f}\" stroke=\"{s}\" \
                             stroke-width=\"1\"/>",
                            f = esc(&color_or(fill, "transparent")),
                            s = esc(&color_or(stroke, "#222")),
                        );
                    }
                    CanvasCommand::Polyline {
                        points,
                        stroke,
                        width,
                    } => {
                        if points.is_empty() {
                            continue;
                        }
                        let pts: String = points
                            .iter()
                            .map(|p| format!("{},{}", p[0], p[1]))
                            .collect::<Vec<_>>()
                            .join(" ");
                        let _ = writeln!(
                            svg,
                            "<polyline points=\"{pts}\" fill=\"none\" stroke=\"{s}\" \
                             stroke-width=\"{wid}\" stroke-linejoin=\"round\" \
                             stroke-linecap=\"round\"/>",
                            s = esc(&color_or(stroke, "#222")),
                            wid = width.unwrap_or(1.5),
                        );
                    }
                    CanvasCommand::Text {
                        x,
                        y,
                        text,
                        fill,
                        size,
                        anchor,
                    } => {
                        let anc = anchor
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| matches!(*s, "start" | "middle" | "end"))
                            .unwrap_or("start");
                        let _ = writeln!(
                            svg,
                            "<text x=\"{x}\" y=\"{y}\" font-size=\"{sz}\" fill=\"{f}\" \
                             text-anchor=\"{anc}\">{lbl}</text>",
                            sz = size.unwrap_or(12.0),
                            f = esc(&color_or(fill, "#111")),
                            lbl = esc(text),
                        );
                    }
                }
            }
            svg.push_str("</svg>");
            let desc = format!(
                "Canvas drawing{} · {} command{} · viewBox {}×{}",
                args.title
                    .as_deref()
                    .map(|t| format!(" \"{}\"", t))
                    .unwrap_or_default(),
                args.commands.len(),
                if args.commands.len() == 1 { "" } else { "s" },
                fmt_tick(w),
                fmt_tick(h),
            );
            Ok(svg_result(svg, desc))
        })
    }
}

// ---------------------------------------------------------------------------
// chart_grafana — dark-themed multi-panel time-series
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GrafanaSeries {
    /// Series label shown in the legend.
    label: String,
    /// `(timestamp_or_x, value)` points. Sorted by x at render time.
    points: Vec<[f64; 2]>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GrafanaArgs {
    /// Panel title shown at the top.
    #[serde(default)]
    title: Option<String>,
    /// One or more series rendered on a single panel with smoothed area
    /// fills behind the strokes.
    series: Vec<GrafanaSeries>,
    /// Optional Y-axis unit label (e.g. "ms", "req/s", "%").
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    width: Option<f64>,
    #[serde(default)]
    height: Option<f64>,
}

pub struct ChartGrafana;
impl Skill for ChartGrafana {
    fn name(&self) -> &'static str {
        "chart_grafana"
    }
    fn description(&self) -> &'static str {
        "Time-series panel rendered in a Grafana-style dark theme: dark slate background, low-\
        contrast grid, multi-series lines with translucent area fills below each, value labels at \
        the latest point, axis ticks formatted compactly. Use for metric / observability dashboards \
        where the Grafana look conveys 'this is operational telemetry'."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GrafanaArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<GrafanaArgs>()?;
            if args.series.is_empty() {
                return Err(invalid(
                    "`series` must contain at least one entry".to_string(),
                ));
            }
            let series_xy: Vec<Vec<(f64, f64)>> = args
                .series
                .iter()
                .map(|s| {
                    let mut pts: Vec<(f64, f64)> = s.points.iter().map(|p| (p[0], p[1])).collect();
                    pts.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                    pts
                })
                .collect();
            for (i, s) in series_xy.iter().enumerate() {
                if s.len() < 2 {
                    return Err(invalid(format!(
                        "series {} (\"{}\") needs at least 2 points",
                        i, args.series[i].label
                    )));
                }
            }
            let w = args.width.unwrap_or(DEFAULT_W).clamp(160.0, 4000.0);
            let h = args.height.unwrap_or(DEFAULT_H).clamp(120.0, 4000.0);
            let m = Margins {
                top: 44.0,
                right: 28.0,
                bottom: 48.0,
                left: 64.0,
            };
            let plot_left = m.left;
            let plot_right = w - m.right;
            let plot_top = m.top;
            let plot_bottom = h - m.bottom;
            let (xmin, xmax, ymin, ymax) = auto_xy_range(&series_xy);
            let x_ticks = nice_ticks(xmin, xmax, 7);
            let y_ticks = nice_ticks(ymin.min(0.0), ymax, 6);
            let xd = (
                *x_ticks.first().unwrap_or(&xmin),
                *x_ticks.last().unwrap_or(&xmax),
            );
            let yd = (
                *y_ticks.first().unwrap_or(&ymin),
                *y_ticks.last().unwrap_or(&ymax),
            );
            // Dark theme override of `svg_open`.
            let mut svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" \
                 preserveAspectRatio=\"xMidYMid meet\" \
                 style=\"font-family:ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif\" \
                 role=\"img\">\n\
                 <rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" fill=\"#181b1f\"/>"
            );
            if let Some(t) = args.title.as_deref() {
                let _ = writeln!(
                    svg,
                    "<text x=\"{x}\" y=\"26\" font-size=\"15\" fill=\"#d8d9da\" \
                     font-weight=\"600\">{lbl}</text>",
                    x = plot_left,
                    lbl = esc(t),
                );
            }
            // Grid + axis ticks, low-contrast lines.
            let scale_x = |v: f64| {
                plot_left + (v - xd.0) / (xd.1 - xd.0).max(f64::EPSILON) * (plot_right - plot_left)
            };
            let scale_y = |v: f64| {
                plot_bottom
                    - (v - yd.0) / (yd.1 - yd.0).max(f64::EPSILON) * (plot_bottom - plot_top)
            };
            for t in &x_ticks {
                let x = scale_x(*t);
                let _ = writeln!(
                    svg,
                    "<line x1=\"{x}\" y1=\"{plot_top}\" x2=\"{x}\" y2=\"{plot_bottom}\" \
                     stroke=\"#2c3036\" stroke-width=\"1\"/>\n\
                     <text x=\"{x}\" y=\"{ly}\" text-anchor=\"middle\" font-size=\"10\" \
                     fill=\"#888\">{lbl}</text>",
                    ly = plot_bottom + 14.0,
                    lbl = esc(&fmt_tick(*t)),
                );
            }
            for t in &y_ticks {
                let y = scale_y(*t);
                let unit = args.unit.as_deref().unwrap_or("");
                let _ = writeln!(
                    svg,
                    "<line x1=\"{plot_left}\" y1=\"{y}\" x2=\"{plot_right}\" y2=\"{y}\" \
                     stroke=\"#2c3036\" stroke-width=\"1\"/>\n\
                     <text x=\"{x}\" y=\"{ty}\" text-anchor=\"end\" font-size=\"10\" \
                     fill=\"#888\">{lbl}{unit}</text>",
                    x = plot_left - 6.0,
                    ty = y + 3.0,
                    lbl = esc(&fmt_tick(*t)),
                );
            }
            // Area fill + line per series.
            for (i, s) in series_xy.iter().enumerate() {
                let color = PALETTE[i % PALETTE.len()];
                let mut path = String::new();
                let mut area = format!("M{:.2},{:.2} ", scale_x(s[0].0), scale_y(yd.0));
                for (j, (x, y)) in s.iter().enumerate() {
                    let cmd = if j == 0 { 'M' } else { 'L' };
                    let _ = write!(path, "{cmd}{:.2},{:.2} ", scale_x(*x), scale_y(*y));
                    let _ = write!(area, "L{:.2},{:.2} ", scale_x(*x), scale_y(*y));
                }
                let _ = write!(
                    area,
                    "L{:.2},{:.2} Z",
                    scale_x(s.last().unwrap().0),
                    scale_y(yd.0)
                );
                let _ = writeln!(
                    svg,
                    "<path d=\"{area}\" fill=\"{color}\" opacity=\"0.15\"/>\n\
                     <path d=\"{path}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\" \
                     stroke-linejoin=\"round\" stroke-linecap=\"round\"/>"
                );
                // Last-value label at the right edge.
                if let Some((lx, ly)) = s.last() {
                    let _ = writeln!(
                        svg,
                        "<circle cx=\"{cx:.2}\" cy=\"{cy:.2}\" r=\"3\" fill=\"{color}\"/>\n\
                         <text x=\"{tx:.2}\" y=\"{ty:.2}\" font-size=\"10\" fill=\"{color}\" \
                         text-anchor=\"end\">{lbl}{unit}</text>",
                        cx = scale_x(*lx),
                        cy = scale_y(*ly),
                        tx = scale_x(*lx) - 6.0,
                        ty = scale_y(*ly) - 6.0,
                        lbl = esc(&fmt_tick(*ly)),
                        unit = esc(args.unit.as_deref().unwrap_or("")),
                    );
                }
            }
            // Dark-themed legend at top-right.
            let mut ly = plot_top - 24.0;
            for (i, s) in args.series.iter().enumerate() {
                let color = PALETTE[i % PALETTE.len()];
                let _ = writeln!(
                    svg,
                    "<rect x=\"{x}\" y=\"{ly}\" width=\"10\" height=\"10\" fill=\"{color}\"/>\n\
                     <text x=\"{tx}\" y=\"{ty}\" font-size=\"11\" fill=\"#bbb\">{lbl}</text>",
                    x = plot_right - 160.0,
                    tx = plot_right - 146.0,
                    ty = ly + 9.0,
                    lbl = esc(&s.label),
                );
                ly += 14.0;
            }
            svg.push_str("</svg>");
            let total_points: usize = series_xy.iter().map(|s| s.len()).sum();
            let desc = format!(
                "Grafana panel{} · {} serie{} · {} points · range x [{}, {}] · y [{}, {}]",
                args.title
                    .as_deref()
                    .map(|t| format!(" \"{}\"", t))
                    .unwrap_or_default(),
                args.series.len(),
                if args.series.len() == 1 { "" } else { "s" },
                total_points,
                fmt_tick(xmin),
                fmt_tick(xmax),
                fmt_tick(ymin),
                fmt_tick(ymax),
            );
            Ok(svg_result(svg, desc))
        })
    }
}

// ---------------------------------------------------------------------------
// chart_interactive — Chart.js / Plotly HTML wrappers (full interactivity
// for clients that render HTML; static fallback for those that don't)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InteractiveArgs {
    /// Charting library to wrap. `"chartjs"` (default, covers line / bar /
    /// radar / pie / doughnut / polarArea / bubble / scatter), `"plotly"`
    /// (covers everything Chart.js does plus 3D, contour, surface, heatmap,
    /// candlestick, sankey, treemap).
    #[serde(default)]
    library: Option<String>,
    /// The library's native config object. For Chart.js: a `{ type, data,
    /// options }` document. For Plotly: a `{ data, layout, config? }`
    /// document. Pass it as plain JSON — we splice it directly into the
    /// page's `<script>` tag.
    config: serde_json::Value,
    /// Optional figure title for the markdown fallback / page `<title>`.
    #[serde(default)]
    title: Option<String>,
    /// Container width in CSS units (`"100%"`, `"640px"`). Default
    /// `"100%"` so the chart is responsive in the embedding viewport.
    #[serde(default)]
    width: Option<String>,
    /// Container height in CSS units. Default `"480px"`.
    #[serde(default)]
    height: Option<String>,
}

pub struct ChartInteractive;
impl Skill for ChartInteractive {
    fn name(&self) -> &'static str {
        "chart_interactive"
    }
    fn description(&self) -> &'static str {
        "Render an interactive chart using Chart.js or Plotly. You provide the library's native \
        config object (Chart.js: `{type, data, options}`; Plotly: `{data, layout, config?}`) and \
        the tool returns a self-contained HTML snippet that loads the library from a CDN and \
        renders it. Clients that render HTML inline (browser-embedded ones, Jupyter, web apps) \
        get full interactivity — hover tooltips, zoom, pan, legend toggling, responsive resize. \
        Clients that show only text/images see a code block with the library and config (useful \
        for export / preview elsewhere). For purely static SVG output, use chart_line / \
        chart_bar / chart_scatter / chart_histogram / chart_heatmap / chart_pie instead."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<InteractiveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<InteractiveArgs>()?;
            let lib = args
                .library
                .as_deref()
                .map(str::trim)
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| "chartjs".to_string());
            let cfg_json = serde_json::to_string(&args.config)
                .map_err(|e| invalid(format!("config must be serializable JSON: {e}")))?;
            let title = args
                .title
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("Interactive chart");
            let css_w = args.width.as_deref().unwrap_or("100%");
            let css_h = args.height.as_deref().unwrap_or("480px");
            let html = match lib.as_str() {
                "chartjs" | "chart.js" => format!(
                    "<!doctype html><html><head><meta charset=\"utf-8\"/><title>{t}</title>\n\
                     <script src=\"https://cdn.jsdelivr.net/npm/chart.js@4\"></script></head>\n\
                     <body style=\"margin:0;padding:8px;font-family:system-ui,sans-serif\">\n\
                     <div style=\"width:{w};height:{h}\"><canvas id=\"c\"></canvas></div>\n\
                     <script>const cfg={cfg};new Chart(document.getElementById('c'),cfg);</script>\n\
                     </body></html>",
                    t = esc(title), w = css_w, h = css_h, cfg = cfg_json,
                ),
                "plotly" => format!(
                    "<!doctype html><html><head><meta charset=\"utf-8\"/><title>{t}</title>\n\
                     <script src=\"https://cdn.plot.ly/plotly-2.35.2.min.js\"></script></head>\n\
                     <body style=\"margin:0;padding:8px;font-family:system-ui,sans-serif\">\n\
                     <div id=\"p\" style=\"width:{w};height:{h}\"></div>\n\
                     <script>const cfg={cfg};Plotly.newPlot('p',cfg.data,cfg.layout||{{}},cfg.config||{{responsive:true}});</script>\n\
                     </body></html>",
                    t = esc(title), w = css_w, h = css_h, cfg = cfg_json,
                ),
                other => {
                    return Err(invalid(format!(
                        "unknown library \"{other}\" — pick \"chartjs\" or \"plotly\""
                    )))
                }
            };
            // We return BOTH: HTML wrapped as text/html content (clients that
            // can render HTML use it) AND a fenced markdown block for clients
            // that don't, so the config and library are inspectable either way.
            let markdown = format!(
                "Interactive chart — {lib}, title \"{title}\". \
                 Clients that render HTML will display the chart below; \
                 others see this fenced markdown block with the library + \
                 config so it can be saved or exported elsewhere.\n\n\
                 ```html\n{html}\n```\n",
                lib = lib,
            );
            let img = Content::image(b64(html.as_bytes()), "text/html");
            let txt = Content::text(markdown);
            Ok(CallToolResult::success(vec![img, txt]))
        })
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(ChartLine),
        Box::new(ChartBar),
        Box::new(ChartScatter),
        Box::new(ChartHistogram),
        Box::new(ChartPie),
        Box::new(ChartHeatmap),
        Box::new(ChartCanvas),
        Box::new(ChartGrafana),
        Box::new(ChartInteractive),
        Box::new(ChartMermaid),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_round_trip_examples() {
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
        assert_eq!(b64(b"foob"), "Zm9vYg==");
        assert_eq!(b64(b"fooba"), "Zm9vYmE=");
        assert_eq!(b64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn nice_ticks_covers_range() {
        let t = nice_ticks(0.0, 100.0, 5);
        assert!(t.len() >= 3);
        assert!(t.first().copied().unwrap() <= 0.0);
        assert!(t.last().copied().unwrap() >= 100.0);
    }

    #[test]
    fn nice_ticks_handles_degenerate_range() {
        let t = nice_ticks(5.0, 5.0, 5);
        // No panic; produces a non-empty axis so the renderer has something
        // sensible to plot against.
        assert!(t.len() >= 2);
    }

    #[test]
    fn fmt_tick_drops_trailing_zeros() {
        assert_eq!(fmt_tick(1.5), "1.5");
        assert_eq!(fmt_tick(1.0), "1");
        assert_eq!(fmt_tick(0.0), "0");
        assert_eq!(fmt_tick(0.001), "0.001");
    }

    #[test]
    fn esc_escapes_xml_special() {
        assert_eq!(esc("<a&b\"c'd>"), "&lt;a&amp;b&quot;c&#39;d&gt;");
    }

    #[test]
    fn auto_xy_range_combines_all_series() {
        let s = vec![vec![(0.0, 1.0), (3.0, 4.0)], vec![(-1.0, 2.0), (2.0, 7.0)]];
        let (xmin, xmax, ymin, ymax) = auto_xy_range(&s);
        assert_eq!(xmin, -1.0);
        assert_eq!(xmax, 3.0);
        assert_eq!(ymin, 1.0);
        assert_eq!(ymax, 7.0);
    }
}
