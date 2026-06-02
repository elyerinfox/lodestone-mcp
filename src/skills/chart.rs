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

use crate::skills::{ensure_min_len, schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

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

/// Bundles the boilerplate every X/Y chart tool used to copy-paste:
/// width / height + margins → plot rectangle, auto-ranged domains from a
/// series, nice ticks, and data-→-pixel scale functions. Replacing this
/// inline block per tool (chart_line, chart_scatter, chart_histogram,
/// chart_candlestick all had it) saves ~20 LoC per call site and
/// guarantees the scaling math is identical everywhere.
struct PlotArea {
    width: f64,
    height: f64,
    margins: Margins,
    x_domain: (f64, f64),
    y_domain: (f64, f64),
    x_ticks: Vec<f64>,
    y_ticks: Vec<f64>,
}

impl PlotArea {
    /// Default margins + 7 X ticks + 6 Y ticks, ranges auto-derived from the
    /// provided XY series.
    fn from_xy(series: &[Vec<(f64, f64)>], width: f64, height: f64) -> Self {
        Self::custom(series, width, height, Margins::default(), 7, 6)
    }

    /// Same shape as [`PlotArea::from_xy`] but with explicit margins / tick
    /// targets. Used by the Grafana-style and heatmap tools that need
    /// custom insets.
    fn custom(
        series: &[Vec<(f64, f64)>],
        width: f64,
        height: f64,
        margins: Margins,
        x_tick_target: usize,
        y_tick_target: usize,
    ) -> Self {
        let (xmin, xmax, ymin, ymax) = auto_xy_range(series);
        Self::from_ranges(
            (xmin, xmax),
            (ymin, ymax),
            width,
            height,
            margins,
            x_tick_target,
            y_tick_target,
        )
    }

    /// Build from caller-supplied data-space ranges (skip the
    /// `auto_xy_range` step). Useful when the caller already knows the
    /// envelope — e.g. histograms (Y is 0..max_count) or bar-style charts.
    fn from_ranges(
        x_range: (f64, f64),
        y_range: (f64, f64),
        width: f64,
        height: f64,
        margins: Margins,
        x_tick_target: usize,
        y_tick_target: usize,
    ) -> Self {
        let x_ticks = nice_ticks(x_range.0, x_range.1, x_tick_target);
        let y_ticks = nice_ticks(y_range.0, y_range.1, y_tick_target);
        let x_domain = (
            *x_ticks.first().unwrap_or(&x_range.0),
            *x_ticks.last().unwrap_or(&x_range.1),
        );
        let y_domain = (
            *y_ticks.first().unwrap_or(&y_range.0),
            *y_ticks.last().unwrap_or(&y_range.1),
        );
        Self {
            width,
            height,
            margins,
            x_domain,
            y_domain,
            x_ticks,
            y_ticks,
        }
    }

    fn left(&self) -> f64 {
        self.margins.left
    }
    fn right(&self) -> f64 {
        self.width - self.margins.right
    }
    fn top(&self) -> f64 {
        self.margins.top
    }
    fn bottom(&self) -> f64 {
        self.height - self.margins.bottom
    }

    fn scale_x(&self, v: f64) -> f64 {
        let (d0, d1) = self.x_domain;
        self.left() + (v - d0) / (d1 - d0).max(f64::EPSILON) * (self.right() - self.left())
    }
    fn scale_y(&self, v: f64) -> f64 {
        let (d0, d1) = self.y_domain;
        self.bottom() - (v - d0) / (d1 - d0).max(f64::EPSILON) * (self.bottom() - self.top())
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

/// Format a chart title as a quoted suffix for the one-line text
/// description, or empty when there's no title. Every chart tool used to
/// inline `title.map(|t| format!(" \"{}\"", t)).unwrap_or_default()` —
/// extracting it removes 13 copies of the same boilerplate.
fn title_suffix(title: Option<&str>) -> String {
    title.map(|t| format!(" \"{}\"", t)).unwrap_or_default()
}

/// Variant of [`svg_open`] for the dark-themed panels (Grafana, Stat,
/// Gauge, Bar Gauge, State Timeline). Opens the SVG and immediately paints
/// a full-canvas background rect in `bg` so subsequent strokes show against
/// the dark backdrop. Centralizes a string literal that used to live
/// inline in five chart tools.
fn svg_open_dark(w: f64, h: f64, bg: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" \
         preserveAspectRatio=\"xMidYMid meet\" \
         style=\"font-family:ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif\" \
         role=\"img\">\n\
         <rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" fill=\"{bg}\"/>",
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

/// Variant of [`render_x_axis`] that uses a pre-computed label per tick
/// (e.g. date-formatted strings). Labels are rotated 30° for readability.
#[allow(clippy::too_many_arguments)]
fn render_x_axis_with_labels(
    out: &mut String,
    ticks: &[f64],
    labels: &[String],
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
    for (t, lbl) in ticks.iter().zip(labels.iter()) {
        let x = scale(*t);
        let _ = writeln!(
            out,
            "<line x1=\"{x}\" y1=\"{plot_top}\" x2=\"{x}\" y2=\"{baseline_y}\" \
             stroke=\"#eee\" stroke-width=\"1\"/>\n\
             <line x1=\"{x}\" y1=\"{baseline_y}\" x2=\"{x}\" y2=\"{tick_top}\" stroke=\"#999\" stroke-width=\"1\"/>\n\
             <text x=\"{x}\" y=\"{label_y}\" text-anchor=\"end\" font-size=\"10\" fill=\"#444\" \
             transform=\"rotate(-30 {x} {label_y})\">{txt}</text>",
            tick_top = baseline_y + 4.0,
            label_y = baseline_y + 18.0,
            txt = esc(lbl),
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

/// A line / scatter / candle data point. JSON shape: a 2-element array.
///
/// **First element (x)** can be a **number** *or* a **string**:
///   - Number → used directly as the x-coordinate.
///   - String → parsed as a date / datetime (ISO-8601 forms accepted:
///     `"2026-01-15"`, `"2026-01-15T12:34:56Z"`, `"2026-01-15 12:34:56"`).
///     The Unix timestamp drives the x-scale; the original string is kept
///     as the rendered tick label, so the axis shows dates instead of
///     timestamps.
///
/// **Second element (y)** must be a number.
///
/// Model-friendly: the JSON schema doesn't strictly type the element,
/// which lets the model emit e.g. `[["2026-01-15", 685.69], …]` without a
/// type-mismatch reject.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LineSeries {
    /// Label shown in the legend.
    label: String,
    /// `[x, y]` data points. `x` is a number OR an ISO-8601 date/datetime
    /// string. `y` is a number. Must contain at least 2 points.
    points: Vec<Vec<serde_json::Value>>,
}

/// Parse a `[x, y]` JSON pair. Returns the numeric coordinates and, when
/// the x was a date string, the original string for use as the tick label.
fn parse_xy(pair: &[serde_json::Value]) -> Option<(f64, f64, Option<String>)> {
    if pair.len() != 2 {
        return None;
    }
    let y = pair[1].as_f64()?;
    if let Some(n) = pair[0].as_f64() {
        return Some((n, y, None));
    }
    let s = pair[0].as_str()?.trim();
    if let Ok(n) = s.parse::<f64>() {
        return Some((n, y, None));
    }
    let ts = parse_date_to_ts(s)?;
    Some((ts, y, Some(s.to_string())))
}

/// Parse a flexible date/datetime string to a Unix timestamp (seconds).
/// Accepts a few common shapes the model is likely to emit:
///   * `YYYY-MM-DD`
///   * `YYYY-MM-DDTHH:MM:SS` and `…HH:MM:SSZ`
///   * `YYYY-MM-DD HH:MM:SS`
///   * `YYYY/MM/DD`
fn parse_date_to_ts(s: &str) -> Option<f64> {
    use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
    let trimmed = s.trim().trim_end_matches('Z');
    let date_formats = ["%Y-%m-%d", "%Y/%m/%d", "%Y-%m"];
    let dt_formats = [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y/%m/%d %H:%M:%S",
    ];
    for f in dt_formats {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(trimmed, f) {
            return Some(Utc.from_utc_datetime(&ndt).timestamp() as f64);
        }
    }
    for f in date_formats {
        if let Ok(nd) = NaiveDate::parse_from_str(trimmed, f) {
            if let Some(ndt) = nd.and_hms_opt(0, 0, 0) {
                return Some(Utc.from_utc_datetime(&ndt).timestamp() as f64);
            }
        }
    }
    None
}

/// Format a Unix timestamp (seconds) as a compact human-readable date.
/// Picks `YYYY-MM-DD` for ranges spanning ≥ 2 days, otherwise adds time.
fn fmt_ts(ts: f64, span_secs: f64) -> String {
    use chrono::TimeZone;
    let dt = chrono::Utc
        .timestamp_opt(ts as i64, 0)
        .single()
        .unwrap_or_else(|| chrono::Utc.timestamp_opt(0, 0).single().unwrap());
    if span_secs >= 2.0 * 86_400.0 {
        dt.format("%Y-%m-%d").to_string()
    } else {
        dt.format("%m-%d %H:%M").to_string()
    }
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
            // Parse points with flexible x (number OR ISO date string).
            // We collect numeric (x, y) for scaling AND a parallel "was the
            // x originally a date string?" flag so the axis can show dates
            // instead of raw seconds.
            let mut series_xy: Vec<Vec<(f64, f64)>> = Vec::with_capacity(args.series.len());
            let mut x_is_date = false;
            for (i, s) in args.series.iter().enumerate() {
                let mut row: Vec<(f64, f64)> = Vec::with_capacity(s.points.len());
                for (j, pt) in s.points.iter().enumerate() {
                    let (x, y, date_label) = parse_xy(pt).ok_or_else(|| {
                        invalid(format!(
                            "series {} (\"{}\") point {}: expected [number-or-date-string, number], \
                             got {}",
                            i,
                            s.label,
                            j,
                            serde_json::to_string(pt).unwrap_or_default()
                        ))
                    })?;
                    if date_label.is_some() {
                        x_is_date = true;
                    }
                    row.push((x, y));
                }
                series_xy.push(row);
            }
            for (i, s) in series_xy.iter().enumerate() {
                ensure_min_len(
                    s,
                    2,
                    &format!("points in series \"{}\"", args.series[i].label),
                )?;
            }
            let w = args.width.unwrap_or(DEFAULT_W).clamp(160.0, 4000.0);
            let h = args.height.unwrap_or(DEFAULT_H).clamp(120.0, 4000.0);
            let pa = PlotArea::from_xy(&series_xy, w, h);
            let mut svg = svg_open(w, h);
            render_chrome(
                &mut svg,
                args.title.as_deref(),
                args.xlabel.as_deref(),
                args.ylabel.as_deref(),
                w,
                h,
                &pa.margins,
            );
            if x_is_date {
                // Re-render the x axis with date-formatted tick labels.
                let span = pa.x_domain.1 - pa.x_domain.0;
                let dated: Vec<String> = pa.x_ticks.iter().map(|t| fmt_ts(*t, span)).collect();
                render_x_axis_with_labels(
                    &mut svg,
                    &pa.x_ticks,
                    &dated,
                    pa.x_domain,
                    (pa.left(), pa.right()),
                    pa.bottom(),
                    pa.top(),
                );
            } else {
                render_x_axis(
                    &mut svg,
                    &pa.x_ticks,
                    pa.x_domain,
                    (pa.left(), pa.right()),
                    pa.bottom(),
                    pa.top(),
                );
            }
            render_y_axis(
                &mut svg,
                &pa.y_ticks,
                pa.y_domain,
                (pa.top(), pa.bottom()),
                pa.left(),
                pa.right(),
            );
            for (i, s) in series_xy.iter().enumerate() {
                let color = PALETTE[i % PALETTE.len()];
                let mut path = String::new();
                for (j, (x, y)) in s.iter().enumerate() {
                    let cmd = if j == 0 { 'M' } else { 'L' };
                    let _ = write!(path, "{cmd}{:.2},{:.2} ", pa.scale_x(*x), pa.scale_y(*y));
                }
                let _ = writeln!(
                    svg,
                    "<path d=\"{path}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"2\" \
                     stroke-linejoin=\"round\" stroke-linecap=\"round\"/>"
                );
            }
            let labels: Vec<&str> = args.series.iter().map(|s| s.label.as_str()).collect();
            render_legend(&mut svg, &labels, PALETTE, pa.right(), pa.top());
            svg.push_str("</svg>");
            let (xmin, xmax) = pa.x_domain;
            let (ymin, ymax) = pa.y_domain;
            let total_points: usize = series_xy.iter().map(|s| s.len()).sum();
            let desc = format!(
                "Line chart{} · {} serie{} · {} points · x ∈ [{}, {}] · y ∈ [{}, {}]",
                title_suffix(args.title.as_deref()),
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Single numeric series",
                args: r#"{"series": [{"label": "load", "points": [[0, 0.2], [1, 0.5], [2, 0.7], [3, 0.4]]}], "title": "CPU load"}"#,
                note: Some("Numeric x; one series, no legend."),
            },
            SkillExample {
                title: "Two series with date-string x",
                args: r#"{"series": [{"label": "A", "points": [["2026-01-01", 10], ["2026-02-01", 18], ["2026-03-01", 22]]}, {"label": "B", "points": [["2026-01-01", 5], ["2026-02-01", 9], ["2026-03-01", 14]]}], "title": "Q1 sales"}"#,
                note: Some("ISO-8601 strings trigger date-formatted x ticks."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Multi-series time-series plot with auto-formatted date axis.",
            "Compare numeric curves on a single shared axis with a legend.",
            "Render any (x, y) trace where interpolation between samples is appropriate.",
        ]
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
            let max_v = args
                .values
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            let min_v = args.values.iter().copied().fold(f64::INFINITY, f64::min);
            // Bars use categorical x (0..n), but we route through PlotArea
            // for the y-axis scaling + nice ticks - the x-axis is drawn
            // manually since there's no continuous x range to tick.
            let n = args.labels.len() as f64;
            let pa = PlotArea::from_ranges(
                (0.0, n),
                (min_v.min(0.0), max_v.max(0.0)),
                w,
                h,
                Margins::default(),
                0,
                6,
            );
            let mut svg = svg_open(w, h);
            render_chrome(
                &mut svg,
                args.title.as_deref(),
                args.xlabel.as_deref(),
                args.ylabel.as_deref(),
                w,
                h,
                &pa.margins,
            );
            render_y_axis(
                &mut svg,
                &pa.y_ticks,
                pa.y_domain,
                (pa.top(), pa.bottom()),
                pa.left(),
                pa.right(),
            );
            let band = (pa.right() - pa.left()) / n;
            let bar_w = band * 0.7;
            let baseline_y = if pa.y_domain.0 <= 0.0 && pa.y_domain.1 >= 0.0 {
                pa.scale_y(0.0)
            } else {
                pa.bottom()
            };
            let _ = writeln!(
                svg,
                "<line x1=\"{left}\" y1=\"{baseline_y}\" x2=\"{right}\" \
                 y2=\"{baseline_y}\" stroke=\"#999\" stroke-width=\"1\"/>",
                left = pa.left(),
                right = pa.right(),
            );
            for (i, (lbl, v)) in args.labels.iter().zip(args.values.iter()).enumerate() {
                let cx = pa.left() + (i as f64 + 0.5) * band;
                let y_v = pa.scale_y(*v);
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
                    ly = pa.bottom() + 16.0,
                    txt = esc(lbl),
                );
            }
            svg.push_str("</svg>");
            let total: f64 = args.values.iter().sum();
            let desc = format!(
                "Bar chart{} · {} bars · sum {} · min {} · max {}",
                title_suffix(args.title.as_deref()),
                args.values.len(),
                fmt_tick(total),
                fmt_tick(min_v),
                fmt_tick(max_v),
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Categorical counts",
                args: r#"{"labels": ["A", "B", "C", "D"], "values": [12, 19, 7, 4], "title": "Issues by team"}"#,
                note: Some("Bars rendered left-to-right in input order."),
            },
            SkillExample {
                title: "Mixed positive / negative",
                args: r#"{"labels": ["jan", "feb", "mar"], "values": [-2.5, 1.0, 3.2], "ylabel": "delta"}"#,
                note: Some("Bars on either side of a zero baseline."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compare a small set of categorical values at a glance.",
            "Show period-over-period deltas with signed bars.",
            "Render any single-series bar plot from labels + values.",
        ]
    }
}

// ---------------------------------------------------------------------------
// chart_scatter
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ScatterArgs {
    /// `[x, y]` points. `x` is a number OR an ISO-8601 date string (same
    /// flexible shape as `chart_line.series[*].points`). `y` is a number.
    points: Vec<Vec<serde_json::Value>>,
    /// Title above the plot.
    #[serde(default)]
    title: Option<String>,
    /// X-axis label.
    #[serde(default)]
    xlabel: Option<String>,
    /// Y-axis label.
    #[serde(default)]
    ylabel: Option<String>,
    /// Plot width in user-space units. Default 760, capped at 4000.
    #[serde(default)]
    width: Option<f64>,
    /// Plot height. Default 460, capped at 4000.
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
        "Render `[x, y]` points as a scatter plot. `x` is a number OR an ISO-8601 date string (same \
        flexible shape as chart_line), `y` is a number — date axes auto-format their ticks. Useful \
        for showing distributions / correlations without committing to an interpolation between \
        samples. Returns SVG with a viewBox."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ScatterArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ScatterArgs>()?;
            ensure_min_len(&args.points, 1, "points")?;
            // Same flexible point parsing as chart_line: x can be a number
            // OR a date string. Tracks `x_is_date` so the axis gets date
            // tick labels when appropriate.
            let mut points_xy: Vec<(f64, f64)> = Vec::with_capacity(args.points.len());
            let mut x_is_date = false;
            for (i, pt) in args.points.iter().enumerate() {
                let (x, y, date_label) = parse_xy(pt).ok_or_else(|| {
                    invalid(format!(
                        "point {i}: expected [number-or-date-string, number], got {}",
                        serde_json::to_string(pt).unwrap_or_default()
                    ))
                })?;
                if date_label.is_some() {
                    x_is_date = true;
                }
                points_xy.push((x, y));
            }
            let w = args.width.unwrap_or(DEFAULT_W).clamp(160.0, 4000.0);
            let h = args.height.unwrap_or(DEFAULT_H).clamp(120.0, 4000.0);
            let r = args.point_size.unwrap_or(4.0).clamp(1.0, 20.0);
            let series: Vec<Vec<(f64, f64)>> = vec![points_xy.clone()];
            let pa = PlotArea::from_xy(&series, w, h);
            let mut svg = svg_open(w, h);
            render_chrome(
                &mut svg,
                args.title.as_deref(),
                args.xlabel.as_deref(),
                args.ylabel.as_deref(),
                w,
                h,
                &pa.margins,
            );
            if x_is_date {
                let span = pa.x_domain.1 - pa.x_domain.0;
                let dated: Vec<String> = pa.x_ticks.iter().map(|t| fmt_ts(*t, span)).collect();
                render_x_axis_with_labels(
                    &mut svg,
                    &pa.x_ticks,
                    &dated,
                    pa.x_domain,
                    (pa.left(), pa.right()),
                    pa.bottom(),
                    pa.top(),
                );
            } else {
                render_x_axis(
                    &mut svg,
                    &pa.x_ticks,
                    pa.x_domain,
                    (pa.left(), pa.right()),
                    pa.bottom(),
                    pa.top(),
                );
            }
            render_y_axis(
                &mut svg,
                &pa.y_ticks,
                pa.y_domain,
                (pa.top(), pa.bottom()),
                pa.left(),
                pa.right(),
            );
            for (x, y) in &points_xy {
                let _ = writeln!(
                    svg,
                    "<circle cx=\"{cx:.2}\" cy=\"{cy:.2}\" r=\"{r}\" fill=\"{c}\" \
                     opacity=\"0.7\"/>",
                    cx = pa.scale_x(*x),
                    cy = pa.scale_y(*y),
                    c = PALETTE[0],
                );
            }
            svg.push_str("</svg>");
            let (xmin, xmax) = pa.x_domain;
            let (ymin, ymax) = pa.y_domain;
            let desc = format!(
                "Scatter{} · {} points · x ∈ [{}, {}] · y ∈ [{}, {}]",
                title_suffix(args.title.as_deref()),
                points_xy.len(),
                fmt_tick(xmin),
                fmt_tick(xmax),
                fmt_tick(ymin),
                fmt_tick(ymax),
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Numeric scatter",
                args: r#"{"points": [[1, 2.1], [1.5, 2.4], [2, 3.0], [2.5, 2.8], [3, 4.1]], "xlabel": "x", "ylabel": "y"}"#,
                note: Some("Each point rendered as a translucent dot."),
            },
            SkillExample {
                title: "Date-axis scatter, bigger markers",
                args: r#"{"points": [["2026-01-01", 100], ["2026-01-15", 140], ["2026-02-01", 90]], "point_size": 8}"#,
                note: Some("Date strings make the axis format human-readable dates."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Show correlation / distribution of unconnected samples.",
            "Plot individual measurements without implying interpolation.",
            "Visualize a date-indexed sparse series.",
        ]
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
    /// Title above the plot.
    #[serde(default)]
    title: Option<String>,
    /// X-axis label.
    #[serde(default)]
    xlabel: Option<String>,
    /// Y-axis label.
    #[serde(default)]
    ylabel: Option<String>,
    /// Plot width in user-space units. Default 760, capped at 4000.
    #[serde(default)]
    width: Option<f64>,
    /// Plot height. Default 460, capped at 4000.
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
            let max_count = *counts.iter().max().unwrap_or(&1) as f64;
            let pa = PlotArea::from_ranges(
                (vmin, vmax),
                (0.0, max_count),
                w,
                h,
                Margins::default(),
                7,
                6,
            );
            let mut svg = svg_open(w, h);
            render_chrome(
                &mut svg,
                args.title.as_deref(),
                args.xlabel.as_deref(),
                args.ylabel.as_deref(),
                w,
                h,
                &pa.margins,
            );
            render_x_axis(
                &mut svg,
                &pa.x_ticks,
                pa.x_domain,
                (pa.left(), pa.right()),
                pa.bottom(),
                pa.top(),
            );
            render_y_axis(
                &mut svg,
                &pa.y_ticks,
                pa.y_domain,
                (pa.top(), pa.bottom()),
                pa.left(),
                pa.right(),
            );
            for (i, c) in counts.iter().enumerate() {
                let lo = vmin + i as f64 * width;
                let hi = lo + width;
                let x_l = pa.scale_x(lo);
                let x_h = pa.scale_x(hi);
                let y_t = pa.scale_y(*c as f64);
                let bw = (x_h - x_l - 1.0).max(0.5);
                let _ = writeln!(
                    svg,
                    "<rect x=\"{x_l:.2}\" y=\"{y_t:.2}\" width=\"{bw:.2}\" height=\"{bh:.2}\" \
                     fill=\"{col}\" opacity=\"0.85\"/>",
                    bh = pa.bottom() - y_t,
                    col = PALETTE[0],
                );
            }
            svg.push_str("</svg>");
            let mean = args.values.iter().sum::<f64>() / args.values.len() as f64;
            let desc = format!(
                "Histogram{} · n = {} · {} bins · range [{}, {}] · mean {}",
                title_suffix(args.title.as_deref()),
                args.values.len(),
                bins,
                fmt_tick(vmin),
                fmt_tick(vmax),
                fmt_tick(mean),
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Auto bins",
                args: r#"{"values": [1.0, 1.5, 1.7, 2.0, 2.2, 2.4, 2.5, 2.8, 3.0, 3.2, 3.5, 4.0], "title": "Latencies"}"#,
                note: Some("`bins` defaults to sqrt(n), clamped to [5, 60]."),
            },
            SkillExample {
                title: "Explicit 20 bins",
                args: r#"{"values": [0.1, 0.2, 0.5, 0.8, 1.2, 1.5, 1.8, 2.1, 2.4, 2.7], "bins": 20, "xlabel": "ms"}"#,
                note: Some("Pass `bins` when you know the resolution you want."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Show the distribution shape of a numeric sample.",
            "Spot multi-modal data before fitting a model.",
            "Visualize a latency / response-time distribution.",
        ]
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
    /// Title above the chart.
    #[serde(default)]
    title: Option<String>,
    /// Plot width in user-space units. Default 760, capped at 4000.
    #[serde(default)]
    width: Option<f64>,
    /// Plot height. Default 460, capped at 4000.
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
                title_suffix(args.title.as_deref()),
                args.slices.len(),
                fmt_tick(total),
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Market share",
                args: r#"{"slices": [{"label": "Chrome", "value": 65}, {"label": "Safari", "value": 19}, {"label": "Firefox", "value": 8}, {"label": "Other", "value": 8}], "title": "Browser share"}"#,
                note: Some("Legend shows label and percentage."),
            },
            SkillExample {
                title: "Two-slice split",
                args: r#"{"slices": [{"label": "pass", "value": 87}, {"label": "fail", "value": 13}]}"#,
                note: Some("Title is optional."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Show proportional composition of a small categorical set.",
            "Visualize percentage breakdowns that sum to 100.",
            "Quick share-of-total view for a presentation.",
        ]
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Flowchart",
                args: r#"{"source": "flowchart LR\n A[Start] --> B{Ok?}\n B -- yes --> C[Done]\n B -- no  --> D[Retry]"}"#,
                note: Some("Returns a markdown ```mermaid code fence."),
            },
            SkillExample {
                title: "Sequence diagram with caption",
                args: r#"{"source": "sequenceDiagram\n Alice->>Bob: ping\n Bob-->>Alice: pong", "title": "Health check"}"#,
                note: Some("Title is rendered as a bold caption above the block."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Author flowcharts, sequence, class, ER, gantt, mindmap diagrams as code.",
            "Embed a diagram that the client can re-theme natively, not a server-rasterized image.",
            "Diagram types beyond the built-in chart_* set (state machines, ER schemas, etc).",
        ]
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
    /// Plot width in user-space units. Default 760, capped at 4000.
    #[serde(default)]
    width: Option<f64>,
    /// Plot height. Default 460, capped at 4000.
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
                title_suffix(args.title.as_deref()),
                nrows,
                ncols,
                fmt_tick(vmin),
                fmt_tick(vmax),
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Confusion matrix",
                args: r#"{"matrix": [[50, 2, 1], [3, 45, 2], [0, 4, 48]], "row_labels": ["A", "B", "C"], "col_labels": ["A", "B", "C"], "title": "Confusion"}"#,
                note: Some("Defaults to the viridis colormap with a colorbar."),
            },
            SkillExample {
                title: "Signed values with diverging colormap",
                args: r#"{"matrix": [[-1.0, 0.0, 1.0], [0.5, -0.5, 0.2]], "colormap": "coolwarm"}"#,
                note: Some("coolwarm centers neutral on the middle of the range."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Confusion / correlation / covariance matrix visualization.",
            "Attention map or any dense 2-D matrix overview.",
            "Image-intensity-like heatmaps for science / ML reports.",
        ]
    }
}

// ---------------------------------------------------------------------------
// chart_canvas — procedural drawing (turtle / Logo / svg primitives)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "lowercase")]
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
                title_suffix(args.title.as_deref()),
                args.commands.len(),
                if args.commands.len() == 1 { "" } else { "s" },
                fmt_tick(w),
                fmt_tick(h),
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Box and a label",
                args: r##"{"commands": [{"kind": "rect", "x": 100, "y": 100, "width": 200, "height": 100, "fill": "#fde68a", "stroke": "#92400e"}, {"kind": "text", "x": 200, "y": 155, "text": "Hello", "anchor": "middle", "size": 24}]}"##,
                note: Some("Coords in user-space; 0,0 = top-left."),
            },
            SkillExample {
                title: "Line / circle / polygon mix",
                args: r##"{"commands": [{"kind": "line", "x1": 0, "y1": 300, "x2": 800, "y2": 300, "stroke": "#888"}, {"kind": "circle", "cx": 400, "cy": 300, "r": 40, "fill": "#3b82f6"}, {"kind": "polygon", "points": [[200, 100], [600, 100], [400, 250]], "fill": "#a7f3d0", "stroke": "#065f46"}]}"##,
                note: Some("Mix and match primitives; commands execute in order."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Custom diagrams that don't fit a standard chart type.",
            "Board layouts, illustrations, sketches over a viewBox.",
            "Programmatic SVG output without writing the markup by hand.",
        ]
    }
}

// ---------------------------------------------------------------------------
// chart_grafana — dark-themed multi-panel time-series
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GrafanaSeries {
    /// Series label shown in the legend.
    label: String,
    /// `[x, y]` points. `x` is a number OR an ISO-8601 date string
    /// (date strings auto-format the axis ticks, which is the natural
    /// shape for time-series telemetry). Sorted by x at render time.
    points: Vec<Vec<serde_json::Value>>,
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
    /// Panel width in user-space units. Default 760, capped at 4000.
    #[serde(default)]
    width: Option<f64>,
    /// Panel height. Default 460, capped at 4000.
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
            ensure_min_len(&args.series, 1, "series")?;
            // Same flexible-x parsing as chart_line / chart_scatter.
            let mut series_xy: Vec<Vec<(f64, f64)>> = Vec::with_capacity(args.series.len());
            let mut x_is_date = false;
            for (i, s) in args.series.iter().enumerate() {
                let mut row: Vec<(f64, f64)> = Vec::with_capacity(s.points.len());
                for (j, pt) in s.points.iter().enumerate() {
                    let (x, y, date_label) = parse_xy(pt).ok_or_else(|| {
                        invalid(format!(
                            "series {} (\"{}\") point {}: expected [number-or-date-string, number], \
                             got {}",
                            i,
                            s.label,
                            j,
                            serde_json::to_string(pt).unwrap_or_default()
                        ))
                    })?;
                    if date_label.is_some() {
                        x_is_date = true;
                    }
                    row.push((x, y));
                }
                row.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                series_xy.push(row);
            }
            for (i, s) in series_xy.iter().enumerate() {
                ensure_min_len(
                    s,
                    2,
                    &format!("points in series \"{}\"", args.series[i].label),
                )?;
            }
            let w = args.width.unwrap_or(DEFAULT_W).clamp(160.0, 4000.0);
            let h = args.height.unwrap_or(DEFAULT_H).clamp(120.0, 4000.0);
            // Grafana uses custom margins to give the legend room at the top.
            let margins = Margins {
                top: 44.0,
                right: 28.0,
                bottom: 48.0,
                left: 64.0,
            };
            // y-axis is clamped to include 0 for the Grafana "from zero" look.
            let (xmin_d, xmax_d, ymin_d, ymax_d) = auto_xy_range(&series_xy);
            let pa = PlotArea::from_ranges(
                (xmin_d, xmax_d),
                (ymin_d.min(0.0), ymax_d),
                w,
                h,
                margins,
                7,
                6,
            );
            let mut svg = svg_open_dark(w, h, "#181b1f");
            if let Some(t) = args.title.as_deref() {
                let _ = writeln!(
                    svg,
                    "<text x=\"{x}\" y=\"26\" font-size=\"15\" fill=\"#d8d9da\" \
                     font-weight=\"600\">{lbl}</text>",
                    x = pa.left(),
                    lbl = esc(t),
                );
            }
            // Grid + axis ticks (low-contrast lines for the operational look).
            // Date strings are formatted via fmt_ts when the series carried them.
            let x_span = pa.x_domain.1 - pa.x_domain.0;
            for t in &pa.x_ticks {
                let x = pa.scale_x(*t);
                let lbl = if x_is_date {
                    fmt_ts(*t, x_span)
                } else {
                    fmt_tick(*t)
                };
                let _ = writeln!(
                    svg,
                    "<line x1=\"{x}\" y1=\"{top}\" x2=\"{x}\" y2=\"{bot}\" \
                     stroke=\"#2c3036\" stroke-width=\"1\"/>\n\
                     <text x=\"{x}\" y=\"{ly}\" text-anchor=\"middle\" font-size=\"10\" \
                     fill=\"#888\">{txt}</text>",
                    top = pa.top(),
                    bot = pa.bottom(),
                    ly = pa.bottom() + 14.0,
                    txt = esc(&lbl),
                );
            }
            for t in &pa.y_ticks {
                let y = pa.scale_y(*t);
                let unit = args.unit.as_deref().unwrap_or("");
                let _ = writeln!(
                    svg,
                    "<line x1=\"{left}\" y1=\"{y}\" x2=\"{right}\" y2=\"{y}\" \
                     stroke=\"#2c3036\" stroke-width=\"1\"/>\n\
                     <text x=\"{x}\" y=\"{ty}\" text-anchor=\"end\" font-size=\"10\" \
                     fill=\"#888\">{lbl}{unit}</text>",
                    left = pa.left(),
                    right = pa.right(),
                    x = pa.left() - 6.0,
                    ty = y + 3.0,
                    lbl = esc(&fmt_tick(*t)),
                );
            }
            // Area fill + line per series.
            let baseline_y_value = pa.y_domain.0;
            for (i, s) in series_xy.iter().enumerate() {
                let color = PALETTE[i % PALETTE.len()];
                let mut path = String::new();
                let mut area = format!(
                    "M{:.2},{:.2} ",
                    pa.scale_x(s[0].0),
                    pa.scale_y(baseline_y_value)
                );
                for (j, (x, y)) in s.iter().enumerate() {
                    let cmd = if j == 0 { 'M' } else { 'L' };
                    let _ = write!(path, "{cmd}{:.2},{:.2} ", pa.scale_x(*x), pa.scale_y(*y));
                    let _ = write!(area, "L{:.2},{:.2} ", pa.scale_x(*x), pa.scale_y(*y));
                }
                let _ = write!(
                    area,
                    "L{:.2},{:.2} Z",
                    pa.scale_x(s.last().unwrap().0),
                    pa.scale_y(baseline_y_value)
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
                        cx = pa.scale_x(*lx),
                        cy = pa.scale_y(*ly),
                        tx = pa.scale_x(*lx) - 6.0,
                        ty = pa.scale_y(*ly) - 6.0,
                        lbl = esc(&fmt_tick(*ly)),
                        unit = esc(args.unit.as_deref().unwrap_or("")),
                    );
                }
            }
            // Dark-themed legend at top-right.
            let mut ly = pa.top() - 24.0;
            for (i, s) in args.series.iter().enumerate() {
                let color = PALETTE[i % PALETTE.len()];
                let _ = writeln!(
                    svg,
                    "<rect x=\"{x}\" y=\"{ly}\" width=\"10\" height=\"10\" fill=\"{color}\"/>\n\
                     <text x=\"{tx}\" y=\"{ty}\" font-size=\"11\" fill=\"#bbb\">{lbl}</text>",
                    x = pa.right() - 160.0,
                    tx = pa.right() - 146.0,
                    ty = ly + 9.0,
                    lbl = esc(&s.label),
                );
                ly += 14.0;
            }
            svg.push_str("</svg>");
            let (xmin, xmax) = pa.x_domain;
            let (ymin, ymax) = pa.y_domain;
            let total_points: usize = series_xy.iter().map(|s| s.len()).sum();
            let desc = format!(
                "Grafana panel{} · {} serie{} · {} points · range x [{}, {}] · y [{}, {}]",
                title_suffix(args.title.as_deref()),
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Single-series latency panel",
                args: r#"{"title": "p99 latency", "series": [{"label": "api", "points": [["2026-06-01T00:00:00Z", 120], ["2026-06-01T00:05:00Z", 135], ["2026-06-01T00:10:00Z", 128], ["2026-06-01T00:15:00Z", 142]]}], "unit": "ms"}"#,
                note: Some("Date strings format the x ticks; `unit` is appended to y labels."),
            },
            SkillExample {
                title: "Two services on one panel",
                args: r#"{"title": "RPS", "series": [{"label": "web", "points": [[0, 220], [1, 240], [2, 235]]}, {"label": "worker", "points": [[0, 80], [1, 90], [2, 92]]}], "unit": "req/s"}"#,
                note: Some("Multiple series get translucent area fills and a top-right legend."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Operational telemetry panel with the recognizable Grafana look.",
            "Metric / observability dashboards rendered server-side.",
            "Time-series with translucent area fills and last-value labels.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Threshold helpers — shared by stat / gauge / bar_gauge / sparkline
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema, Clone)]
struct Threshold {
    /// Value at or above which this band's `color` applies. Lower bands
    /// (smaller `at`) define the base; higher bands take over as `value`
    /// climbs past them. By convention, include one threshold at the
    /// minimum (often `0`) with the "normal" color.
    at: f64,
    /// SVG color (named, `#hex`, `rgb()`). Grafana defaults are green for
    /// normal, yellow / orange for warning, red for critical.
    color: String,
}

fn threshold_color<'a>(value: f64, thresholds: &'a [Threshold], default: &'a str) -> &'a str {
    let mut picked = default;
    for t in thresholds {
        if value >= t.at {
            picked = &t.color;
        }
    }
    picked
}

/// Default Grafana-style thresholds when none are supplied: green at 0,
/// yellow at 60% of `max`, red at 80% of `max`.
fn default_thresholds(_min: f64, max: f64) -> Vec<Threshold> {
    vec![
        Threshold {
            at: f64::NEG_INFINITY,
            color: "#73bf69".into(),
        },
        Threshold {
            at: 0.6 * max,
            color: "#f2cc0c".into(),
        },
        Threshold {
            at: 0.8 * max,
            color: "#e02f44".into(),
        },
    ]
}

/// Inline mini sparkline drawn within `(x0, y0)`→`(x0+w, y0+h)` user-space
/// units. No axes; pure trend shape. Used by both `chart_sparkline` and as
/// the optional inline trend on `chart_stat`.
#[allow(clippy::too_many_arguments)]
fn draw_sparkline(
    out: &mut String,
    points: &[(f64, f64)],
    x0: f64,
    y0: f64,
    w: f64,
    h: f64,
    color: &str,
    fill_opacity: f64,
) {
    if points.len() < 2 {
        return;
    }
    let xs: Vec<f64> = points.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = points.iter().map(|p| p.1).collect();
    let xmin = xs.iter().copied().fold(f64::INFINITY, f64::min);
    let xmax = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let ymin = ys.iter().copied().fold(f64::INFINITY, f64::min);
    let ymax = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let sx = |x: f64| {
        if (xmax - xmin).abs() < f64::EPSILON {
            x0 + w / 2.0
        } else {
            x0 + (x - xmin) / (xmax - xmin) * w
        }
    };
    let sy = |y: f64| {
        if (ymax - ymin).abs() < f64::EPSILON {
            y0 + h / 2.0
        } else {
            y0 + h - (y - ymin) / (ymax - ymin) * h
        }
    };
    let mut path = String::new();
    let mut area = format!("M{:.2},{:.2} ", sx(points[0].0), y0 + h);
    for (i, p) in points.iter().enumerate() {
        let cmd = if i == 0 { 'M' } else { 'L' };
        let _ = write!(path, "{cmd}{:.2},{:.2} ", sx(p.0), sy(p.1));
        let _ = write!(area, "L{:.2},{:.2} ", sx(p.0), sy(p.1));
    }
    let _ = write!(area, "L{:.2},{:.2} Z", sx(points.last().unwrap().0), y0 + h);
    if fill_opacity > 0.0 {
        let _ = writeln!(
            out,
            "<path d=\"{area}\" fill=\"{color}\" opacity=\"{fill_opacity}\"/>"
        );
    }
    let _ = writeln!(
        out,
        "<path d=\"{path}\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1.5\" \
         stroke-linejoin=\"round\" stroke-linecap=\"round\"/>"
    );
}

// ---------------------------------------------------------------------------
// chart_stat — Grafana "Stat" panel: a big number, threshold-tinted, with an
// optional background sparkline. The signature operational dashboard tile.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StatArgs {
    /// The big number to display.
    value: f64,
    /// Optional label shown above the value.
    #[serde(default)]
    label: Option<String>,
    /// Unit suffix appended to the value (e.g. "ms", "%", "req/s").
    #[serde(default)]
    unit: Option<String>,
    /// Decimal places. Default 2.
    #[serde(default)]
    decimals: Option<u32>,
    /// Threshold bands. Maps the value to a color via "highest reached
    /// threshold wins" logic.
    #[serde(default)]
    thresholds: Option<Vec<Threshold>>,
    /// How the threshold color is applied. `"value"` colors just the
    /// number (default), `"background"` colors the entire tile.
    #[serde(default)]
    color_mode: Option<String>,
    /// Optional sparkline drawn behind the number (the trend leading up to
    /// `value`). `(x, y)` points; the renderer auto-fits the bounds.
    #[serde(default)]
    sparkline: Option<Vec<[f64; 2]>>,
    /// Tile width in user-space units. Default 240.
    #[serde(default)]
    width: Option<f64>,
    /// Tile height in user-space units. Default 120.
    #[serde(default)]
    height: Option<f64>,
}

pub struct ChartStat;
impl Skill for ChartStat {
    fn name(&self) -> &'static str {
        "chart_stat"
    }
    fn description(&self) -> &'static str {
        "Grafana 'Stat' panel — one big number, threshold-tinted, with an optional background \
        sparkline showing the trend leading up to it. The most recognizable operational dashboard \
        tile. Pass `color_mode=\"background\"` to flood-fill the tile (the dramatic \
        green/yellow/red status look). `thresholds` is a list of `{at, color}` entries; the \
        highest reached threshold wins. `sparkline` is an optional trail of `(x, y)` points."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StatArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<StatArgs>()?;
            let w = args.width.unwrap_or(360.0).clamp(120.0, 4000.0);
            let h = args.height.unwrap_or(220.0).clamp(80.0, 4000.0);
            let thresholds = args
                .thresholds
                .unwrap_or_else(|| default_thresholds(0.0, 100.0));
            let color = threshold_color(args.value, &thresholds, "#73bf69").to_string();
            let color_mode = args
                .color_mode
                .as_deref()
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| "value".to_string());
            let bg = if color_mode == "background" {
                color.as_str()
            } else {
                "#1f2329"
            };
            let value_color = if color_mode == "background" {
                "#ffffff"
            } else {
                color.as_str()
            };
            let decimals = args.decimals.unwrap_or(2).min(8);
            let formatted = if args.value.is_finite() {
                let s = format!("{:.*}", decimals as usize, args.value);
                // Strip trailing zeros after the decimal — looks cleaner for
                // integers passed in as floats.
                if s.contains('.') {
                    s.trim_end_matches('0').trim_end_matches('.').to_string()
                } else {
                    s
                }
            } else {
                "—".to_string()
            };
            let unit = args.unit.as_deref().unwrap_or("");
            let mut svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" \
                 preserveAspectRatio=\"xMidYMid meet\" \
                 style=\"font-family:ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif\" \
                 role=\"img\">\n\
                 <rect x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\" rx=\"4\" fill=\"{bg}\"/>"
            );
            if let Some(spark) = args.sparkline.as_ref() {
                let pts: Vec<(f64, f64)> = spark.iter().map(|p| (p[0], p[1])).collect();
                let spark_color = if color_mode == "background" {
                    "rgba(255,255,255,0.45)"
                } else {
                    color.as_str()
                };
                let opacity = 0.18_f64;
                draw_sparkline(
                    &mut svg,
                    &pts,
                    8.0,
                    h * 0.55,
                    w - 16.0,
                    h * 0.40,
                    spark_color,
                    opacity,
                );
            }
            if let Some(lbl) = args.label.as_deref() {
                let label_color = if color_mode == "background" {
                    "rgba(255,255,255,0.85)"
                } else {
                    "#bbb"
                };
                let _ = writeln!(
                    svg,
                    "<text x=\"{cx}\" y=\"30\" text-anchor=\"middle\" font-size=\"13\" \
                     fill=\"{label_color}\">{lbl}</text>",
                    cx = w / 2.0,
                    lbl = esc(lbl),
                );
            }
            let _ = writeln!(
                svg,
                "<text x=\"{cx}\" y=\"{cy}\" text-anchor=\"middle\" font-size=\"{fs}\" \
                 fill=\"{value_color}\" font-weight=\"700\">{val}<tspan font-size=\"{us}\" \
                 font-weight=\"500\" dx=\"4\">{unit}</tspan></text>",
                cx = w / 2.0,
                cy = h * 0.55,
                fs = h * 0.32,
                val = esc(&formatted),
                us = h * 0.13,
                unit = esc(unit),
            );
            svg.push_str("</svg>");
            let desc = format!(
                "Stat panel{} · value {}{unit}{}",
                args.label
                    .as_deref()
                    .map(|t| format!(" \"{}\"", t))
                    .unwrap_or_default(),
                formatted,
                if args.sparkline.is_some() {
                    " · with sparkline"
                } else {
                    ""
                },
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Plain stat tile",
                args: r#"{"value": 87.4, "label": "CPU", "unit": "%", "decimals": 1}"#,
                note: Some("Defaults to value-tinted color and dark tile background."),
            },
            SkillExample {
                title: "Background color mode with sparkline",
                args: r##"{"value": 250, "label": "p99", "unit": "ms", "color_mode": "background", "sparkline": [[0, 200], [1, 220], [2, 240], [3, 250]], "thresholds": [{"at": 0, "color": "#73bf69"}, {"at": 200, "color": "#f2cc0c"}, {"at": 300, "color": "#e02f44"}]}"##,
                note: Some("background mode flood-fills the tile in the threshold color."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Single-number SLO / KPI tile on a dashboard.",
            "Health summary with green/yellow/red status background.",
            "Tile showing 'current value' with a trail sparkline behind it.",
        ]
    }
}

// ---------------------------------------------------------------------------
// chart_gauge — Grafana radial gauge with threshold bands
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GaugeArgs {
    /// Current value displayed by the gauge.
    value: f64,
    /// Lower bound of the gauge range.
    min: f64,
    /// Upper bound of the gauge range (must be greater than `min`).
    max: f64,
    /// Optional title above the gauge.
    #[serde(default)]
    title: Option<String>,
    /// Unit suffix appended to the value (e.g. "%", "ms").
    #[serde(default)]
    unit: Option<String>,
    /// Threshold bands that tint the arc. Defaults to a sensible 3-band split.
    #[serde(default)]
    thresholds: Option<Vec<Threshold>>,
    /// Decimal places in the readout. Default 2.
    #[serde(default)]
    decimals: Option<u32>,
    /// Gauge width in user-space units. Default 320.
    #[serde(default)]
    width: Option<f64>,
    /// Gauge height in user-space units. Default 260.
    #[serde(default)]
    height: Option<f64>,
}

/// Polar-to-cartesian, with 0° at 9 o'clock and angle sweeping clockwise.
fn polar(cx: f64, cy: f64, r: f64, theta: f64) -> (f64, f64) {
    (cx + r * theta.cos(), cy + r * theta.sin())
}

pub struct ChartGauge;
impl Skill for ChartGauge {
    fn name(&self) -> &'static str {
        "chart_gauge"
    }
    fn description(&self) -> &'static str {
        "Grafana 'Gauge' panel — a 270° radial dial showing `value` between `min` and `max`. \
        Threshold bands tint the arc; the needle / fill terminates at `value`'s angle. Numerical \
        readout in the middle. Common for SLO / latency / utilization dashboards."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GaugeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<GaugeArgs>()?;
            if args.max <= args.min {
                return Err(invalid("`max` must be greater than `min`".to_string()));
            }
            let w = args.width.unwrap_or(320.0).clamp(120.0, 4000.0);
            let h = args.height.unwrap_or(260.0).clamp(120.0, 4000.0);
            let thresholds = args
                .thresholds
                .unwrap_or_else(|| default_thresholds(args.min, args.max));
            let cx = w / 2.0;
            let cy = h * 0.62;
            let r = (w.min(h * 1.4)) * 0.38;
            let stroke = r * 0.28;
            // Sweep arc from -225° to +45° (270° total, 12 o'clock = -90°).
            let start = -225f64.to_radians();
            let end = 45f64.to_radians();
            let total = end - start;
            // Background arc.
            let (sx, sy) = polar(cx, cy, r, start);
            let (ex, ey) = polar(cx, cy, r, end);
            let mut svg = svg_open_dark(w, h, "#1f2329");
            let _ = writeln!(
                svg,
                "<path d=\"M{sx:.2},{sy:.2} A{r:.2},{r:.2} 0 1 1 {ex:.2},{ey:.2}\" \
                 fill=\"none\" stroke=\"#2c3036\" stroke-width=\"{stroke}\" stroke-linecap=\"butt\"/>"
            );
            // Threshold band-colored fill from start to the value's angle.
            let frac = ((args.value - args.min) / (args.max - args.min)).clamp(0.0, 1.0);
            let value_end = start + total * frac;
            if frac > 0.0 {
                let (vx, vy) = polar(cx, cy, r, value_end);
                let large = if total * frac > std::f64::consts::PI {
                    1
                } else {
                    0
                };
                let color = threshold_color(args.value, &thresholds, "#73bf69");
                let _ = writeln!(
                    svg,
                    "<path d=\"M{sx:.2},{sy:.2} A{r:.2},{r:.2} 0 {large} 1 {vx:.2},{vy:.2}\" \
                     fill=\"none\" stroke=\"{color}\" stroke-width=\"{stroke}\" stroke-linecap=\"butt\"/>"
                );
            }
            // Tick marks at min / mid / max.
            for (frac_t, lbl) in [
                (0.0, fmt_tick(args.min)),
                (0.5, fmt_tick((args.min + args.max) / 2.0)),
                (1.0, fmt_tick(args.max)),
            ] {
                let theta = start + total * frac_t;
                let (tx, ty) = polar(cx, cy, r + stroke * 0.7, theta);
                let _ = writeln!(
                    svg,
                    "<text x=\"{tx:.2}\" y=\"{ty:.2}\" text-anchor=\"middle\" font-size=\"11\" \
                     fill=\"#888\">{lbl}</text>"
                );
            }
            // Big value in the middle.
            let decimals = args.decimals.unwrap_or(2).min(8);
            let val_str = format!("{:.*}", decimals as usize, args.value);
            let val_str = if val_str.contains('.') {
                val_str
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string()
            } else {
                val_str
            };
            let _ = writeln!(
                svg,
                "<text x=\"{cx:.2}\" y=\"{cy:.2}\" text-anchor=\"middle\" font-size=\"{fs:.2}\" \
                 fill=\"#fff\" font-weight=\"700\">{v}<tspan font-size=\"{us:.2}\" \
                 font-weight=\"500\" dx=\"3\">{unit}</tspan></text>",
                fs = r * 0.55,
                us = r * 0.22,
                v = esc(&val_str),
                unit = esc(args.unit.as_deref().unwrap_or("")),
            );
            if let Some(t) = args.title.as_deref() {
                let _ = writeln!(
                    svg,
                    "<text x=\"{cx}\" y=\"22\" text-anchor=\"middle\" font-size=\"13\" \
                     fill=\"#bbb\" font-weight=\"600\">{lbl}</text>",
                    lbl = esc(t),
                );
            }
            svg.push_str("</svg>");
            let desc = format!(
                "Gauge{} · {} of [{}, {}] ({:.1}%)",
                title_suffix(args.title.as_deref()),
                fmt_tick(args.value),
                fmt_tick(args.min),
                fmt_tick(args.max),
                frac * 100.0,
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Percent utilization",
                args: r#"{"value": 72, "min": 0, "max": 100, "unit": "%", "title": "Disk"}"#,
                note: Some("Uses default green/yellow/red thresholds at 0 / 60% / 80% of max."),
            },
            SkillExample {
                title: "Custom threshold bands",
                args: r##"{"value": 8.5, "min": 0, "max": 10, "unit": "", "thresholds": [{"at": 0, "color": "#73bf69"}, {"at": 7, "color": "#f2cc0c"}, {"at": 9, "color": "#e02f44"}]}"##,
                note: Some("Threshold colors override the defaults for the arc band."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "SLO / utilization gauge with min/max bounds.",
            "Latency / saturation single-number dial.",
            "Bounded percentage indicator for a dashboard tile.",
        ]
    }
}

// ---------------------------------------------------------------------------
// chart_bar_gauge — Grafana horizontal threshold bars (one row per item)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BarGaugeItem {
    label: String,
    value: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BarGaugeArgs {
    /// One row per item; each is `{label, value}`.
    items: Vec<BarGaugeItem>,
    /// Lower bound applied to every bar.
    min: f64,
    /// Upper bound applied to every bar (must be greater than `min`).
    max: f64,
    /// Optional title above the panel.
    #[serde(default)]
    title: Option<String>,
    /// Unit suffix appended to each value (e.g. "%", "ms").
    #[serde(default)]
    unit: Option<String>,
    /// Threshold bands that tint each bar. Defaults to a sensible 3-band split.
    #[serde(default)]
    thresholds: Option<Vec<Threshold>>,
    /// Panel width in user-space units.
    #[serde(default)]
    width: Option<f64>,
    /// Panel height in user-space units.
    #[serde(default)]
    height: Option<f64>,
}

pub struct ChartBarGauge;
impl Skill for ChartBarGauge {
    fn name(&self) -> &'static str {
        "chart_bar_gauge"
    }
    fn description(&self) -> &'static str {
        "Grafana 'Bar gauge' panel — one horizontal bar per item, filled proportionally to \
        `(value - min) / (max - min)` and tinted by the highest reached threshold. The numerical \
        readout sits on the right. Common for compact 'all my hosts' or 'top N pods' tiles."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BarGaugeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<BarGaugeArgs>()?;
            if args.items.is_empty() {
                return Err(invalid(
                    "`items` must contain at least one entry".to_string(),
                ));
            }
            if args.max <= args.min {
                return Err(invalid("`max` must be greater than `min`".to_string()));
            }
            let n = args.items.len();
            let row_h = 28.0_f64;
            let w = args.width.unwrap_or(520.0).clamp(160.0, 4000.0);
            let h = args
                .height
                .unwrap_or((n as f64 * (row_h + 8.0) + 56.0).max(120.0))
                .clamp(80.0, 8000.0);
            let thresholds = args
                .thresholds
                .unwrap_or_else(|| default_thresholds(args.min, args.max));
            let label_w = 120.0_f64;
            let value_w = 80.0_f64;
            let bar_left = label_w + 8.0;
            let bar_right = w - value_w - 8.0;
            let mut svg = svg_open_dark(w, h, "#1f2329");
            if let Some(t) = args.title.as_deref() {
                let _ = writeln!(
                    svg,
                    "<text x=\"12\" y=\"22\" font-size=\"13\" fill=\"#bbb\" font-weight=\"600\">{lbl}</text>",
                    lbl = esc(t),
                );
            }
            let unit = args.unit.as_deref().unwrap_or("");
            let mut y = 40.0_f64;
            for it in &args.items {
                let frac = ((it.value - args.min) / (args.max - args.min)).clamp(0.0, 1.0);
                let color = threshold_color(it.value, &thresholds, "#73bf69");
                let _ = writeln!(
                    svg,
                    "<text x=\"12\" y=\"{ty}\" font-size=\"12\" fill=\"#ddd\">{lbl}</text>\n\
                     <rect x=\"{bar_left}\" y=\"{y}\" width=\"{bw}\" height=\"{row_h}\" rx=\"3\" \
                     fill=\"#2c3036\"/>\n\
                     <rect x=\"{bar_left}\" y=\"{y}\" width=\"{fw}\" height=\"{row_h}\" rx=\"3\" \
                     fill=\"{color}\" opacity=\"0.85\"/>\n\
                     <text x=\"{vx}\" y=\"{ty}\" text-anchor=\"end\" font-size=\"12\" \
                     fill=\"#fff\" font-weight=\"600\">{val}{unit}</text>",
                    ty = y + row_h * 0.65,
                    bw = bar_right - bar_left,
                    fw = (bar_right - bar_left) * frac,
                    vx = w - 12.0,
                    val = esc(&fmt_tick(it.value)),
                    unit = esc(unit),
                    lbl = esc(&it.label),
                );
                y += row_h + 8.0;
            }
            svg.push_str("</svg>");
            let desc = format!(
                "Bar gauge{} · {} items · scale [{}, {}]",
                title_suffix(args.title.as_deref()),
                args.items.len(),
                fmt_tick(args.min),
                fmt_tick(args.max),
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Host utilization rollup",
                args: r#"{"items": [{"label": "web-01", "value": 42}, {"label": "web-02", "value": 78}, {"label": "web-03", "value": 91}], "min": 0, "max": 100, "unit": "%", "title": "CPU"}"#,
                note: Some("Bars sit on a dark track, filled to (value-min)/(max-min)."),
            },
            SkillExample {
                title: "Latencies with custom thresholds",
                args: r##"{"items": [{"label": "api", "value": 120}, {"label": "search", "value": 240}], "min": 0, "max": 500, "unit": "ms", "thresholds": [{"at": 0, "color": "#73bf69"}, {"at": 150, "color": "#f2cc0c"}, {"at": 300, "color": "#e02f44"}]}"##,
                note: Some("Each row colors itself by the highest reached threshold."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Top-N hosts / pods by some metric in a compact panel.",
            "Side-by-side comparison of bounded values across labels.",
            "Quick health snapshot of several services on a single tile.",
        ]
    }
}

// ---------------------------------------------------------------------------
// chart_state_timeline — categorical state over time per row
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StateSegment {
    /// Segment start (x-axis position).
    from: f64,
    /// Segment end (x-axis position). Must be > `from`.
    to: f64,
    /// State label (e.g. "up", "degraded", "down", "scheduled"). Color is
    /// looked up in `state_colors`; unknown states fall back to a default.
    state: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StateRow {
    /// Row label shown on the left.
    label: String,
    /// Time-ordered segments. Don't have to be contiguous; gaps render as
    /// the background.
    segments: Vec<StateSegment>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StateTimelineArgs {
    /// One row per entity tracked.
    rows: Vec<StateRow>,
    /// Map of state name → color. Common defaults applied for known states
    /// (`up`/`ok` = green, `degraded`/`warning` = yellow,
    /// `down`/`error`/`critical` = red, `unknown` = gray).
    #[serde(default)]
    state_colors: Option<std::collections::HashMap<String, String>>,
    /// Optional title above the timeline.
    #[serde(default)]
    title: Option<String>,
    /// Panel width in user-space units.
    #[serde(default)]
    width: Option<f64>,
    /// Panel height in user-space units.
    #[serde(default)]
    height: Option<f64>,
}

fn lookup_state_color(
    state: &str,
    overrides: &Option<std::collections::HashMap<String, String>>,
) -> String {
    let key = state.trim().to_ascii_lowercase();
    if let Some(m) = overrides.as_ref() {
        if let Some(c) = m.get(&key) {
            return c.clone();
        }
    }
    match key.as_str() {
        "up" | "ok" | "good" | "healthy" | "online" | "running" => "#73bf69".into(),
        "degraded" | "warning" | "warn" | "yellow" => "#f2cc0c".into(),
        "down" | "error" | "critical" | "fail" | "failed" | "offline" => "#e02f44".into(),
        "scheduled" | "maintenance" | "paused" => "#a0a0ff".into(),
        "unknown" | "" => "#666".into(),
        _ => "#5794f2".into(),
    }
}

pub struct ChartStateTimeline;
impl Skill for ChartStateTimeline {
    fn name(&self) -> &'static str {
        "chart_state_timeline"
    }
    fn description(&self) -> &'static str {
        "Grafana 'State timeline' panel — one row per series, horizontal colored segments \
        showing categorical state over time. The 'is each service up' grid that everyone uses for \
        SLO reporting. Each row is `{label, segments: [{from, to, state}]}`. State→color map is \
        provided by `state_colors` or falls back to sensible defaults (up=green, degraded=yellow, \
        down=red, scheduled=blue, unknown=gray)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StateTimelineArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<StateTimelineArgs>()?;
            if args.rows.is_empty() {
                return Err(invalid(
                    "`rows` must contain at least one entry".to_string(),
                ));
            }
            // Compute the global x-range from all segments.
            let mut xmin = f64::INFINITY;
            let mut xmax = f64::NEG_INFINITY;
            for r in &args.rows {
                for s in &r.segments {
                    if s.from.is_finite() {
                        xmin = xmin.min(s.from);
                    }
                    if s.to.is_finite() {
                        xmax = xmax.max(s.to);
                    }
                }
            }
            if !xmin.is_finite() {
                xmin = 0.0;
                xmax = 1.0;
            }
            if (xmax - xmin).abs() < f64::EPSILON {
                xmax = xmin + 1.0;
            }
            let row_h = 26.0_f64;
            let n = args.rows.len();
            let w = args.width.unwrap_or(760.0).clamp(200.0, 4000.0);
            let h = args
                .height
                .unwrap_or((n as f64 * (row_h + 8.0) + 76.0).max(140.0))
                .clamp(120.0, 8000.0);
            let label_w = 140.0_f64;
            let plot_left = label_w + 8.0;
            let plot_right = w - 16.0;
            let plot_top = 44.0_f64;
            let plot_bottom = h - 32.0;
            let scale_x =
                |x: f64| plot_left + (x - xmin) / (xmax - xmin) * (plot_right - plot_left);
            let mut svg = svg_open_dark(w, h, "#1f2329");
            if let Some(t) = args.title.as_deref() {
                let _ = writeln!(
                    svg,
                    "<text x=\"12\" y=\"24\" font-size=\"14\" fill=\"#bbb\" font-weight=\"600\">{lbl}</text>",
                    lbl = esc(t),
                );
            }
            // Background row stripes for visual rhythm.
            let mut y = plot_top;
            let height_per_row = (plot_bottom - plot_top) / n as f64;
            for (i, row) in args.rows.iter().enumerate() {
                let bg = if i % 2 == 0 { "#23282d" } else { "#1f2329" };
                let _ = writeln!(
                    svg,
                    "<rect x=\"{plot_left}\" y=\"{y}\" width=\"{rw}\" height=\"{row_h}\" \
                     fill=\"{bg}\" opacity=\"0.6\"/>\n\
                     <text x=\"12\" y=\"{ly}\" font-size=\"12\" fill=\"#ddd\">{lbl}</text>",
                    rw = plot_right - plot_left,
                    ly = y + row_h * 0.65,
                    lbl = esc(&row.label),
                );
                for seg in &row.segments {
                    if !seg.to.is_finite() || !seg.from.is_finite() || seg.to <= seg.from {
                        continue;
                    }
                    let x1 = scale_x(seg.from);
                    let x2 = scale_x(seg.to);
                    let color = lookup_state_color(&seg.state, &args.state_colors);
                    let _ = writeln!(
                        svg,
                        "<rect x=\"{x1:.2}\" y=\"{y:.2}\" width=\"{w:.2}\" height=\"{row_h}\" \
                         fill=\"{color}\" opacity=\"0.9\"/>",
                        w = x2 - x1
                    );
                }
                y += height_per_row;
            }
            // X-axis ticks (5 evenly spaced).
            let ticks = nice_ticks(xmin, xmax, 5);
            for t in &ticks {
                let x = scale_x(*t);
                let _ = writeln!(
                    svg,
                    "<line x1=\"{x:.2}\" y1=\"{plot_bottom}\" x2=\"{x:.2}\" y2=\"{ty:.2}\" \
                     stroke=\"#3a3f47\" stroke-width=\"1\"/>\n\
                     <text x=\"{x:.2}\" y=\"{ly}\" text-anchor=\"middle\" font-size=\"10\" \
                     fill=\"#888\">{lbl}</text>",
                    ty = plot_bottom + 4.0,
                    ly = plot_bottom + 18.0,
                    lbl = esc(&fmt_tick(*t)),
                );
            }
            svg.push_str("</svg>");
            let total_segments: usize = args.rows.iter().map(|r| r.segments.len()).sum();
            let desc = format!(
                "State timeline{} · {} row{} · {} segments · x ∈ [{}, {}]",
                title_suffix(args.title.as_deref()),
                args.rows.len(),
                if args.rows.len() == 1 { "" } else { "s" },
                total_segments,
                fmt_tick(xmin),
                fmt_tick(xmax),
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Two services, uptime grid",
                args: r#"{"rows": [{"label": "web", "segments": [{"from": 0, "to": 50, "state": "up"}, {"from": 50, "to": 60, "state": "degraded"}, {"from": 60, "to": 100, "state": "up"}]}, {"label": "db", "segments": [{"from": 0, "to": 80, "state": "up"}, {"from": 80, "to": 100, "state": "down"}]}], "title": "Status"}"#,
                note: Some("Known states (up/degraded/down/scheduled/unknown) are auto-colored."),
            },
            SkillExample {
                title: "Custom state colors",
                args: r##"{"rows": [{"label": "deploy", "segments": [{"from": 0, "to": 30, "state": "queued"}, {"from": 30, "to": 80, "state": "running"}, {"from": 80, "to": 100, "state": "ok"}]}], "state_colors": {"queued": "#a0a0ff", "running": "#5794f2", "ok": "#73bf69"}}"##,
                note: Some("`state_colors` overrides the default palette for custom labels."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "SLO / uptime grid across hosts or services.",
            "Categorical state over time when value-vs-time would lose meaning.",
            "Deployment / job lifecycle panel with named stages.",
        ]
    }
}

// ---------------------------------------------------------------------------
// chart_candlestick — OHLC candles (financial)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct Candle {
    /// X position (timestamp or index).
    x: f64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CandlestickArgs {
    /// OHLC candles in chronological order.
    candles: Vec<Candle>,
    /// Title above the chart.
    #[serde(default)]
    title: Option<String>,
    /// X-axis label.
    #[serde(default)]
    xlabel: Option<String>,
    /// Y-axis label.
    #[serde(default)]
    ylabel: Option<String>,
    /// Color when close ≥ open. Default `#73bf69` (green).
    #[serde(default)]
    up_color: Option<String>,
    /// Color when close < open. Default `#e02f44` (red).
    #[serde(default)]
    down_color: Option<String>,
    /// Plot width in user-space units. Default 760, capped at 4000.
    #[serde(default)]
    width: Option<f64>,
    /// Plot height. Default 460, capped at 4000.
    #[serde(default)]
    height: Option<f64>,
}

pub struct ChartCandlestick;
impl Skill for ChartCandlestick {
    fn name(&self) -> &'static str {
        "chart_candlestick"
    }
    fn description(&self) -> &'static str {
        "Grafana 'Candlestick' panel — OHLC (open / high / low / close) candles for time-series \
        with open and close per interval. Up candles (close ≥ open) are green, down candles are \
        red. Wicks span low→high; the body spans open↔close. Common for financial / market data."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CandlestickArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<CandlestickArgs>()?;
            ensure_min_len(&args.candles, 1, "candles")?;
            let mut xs: Vec<f64> = args.candles.iter().map(|c| c.x).collect();
            xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let xmin_data = *xs.first().unwrap();
            let xmax_data = *xs.last().unwrap();
            let mut ymin_data = f64::INFINITY;
            let mut ymax_data = f64::NEG_INFINITY;
            for c in &args.candles {
                ymin_data = ymin_data.min(c.low);
                ymax_data = ymax_data.max(c.high);
            }
            let w = args.width.unwrap_or(DEFAULT_W).clamp(160.0, 4000.0);
            let h = args.height.unwrap_or(DEFAULT_H).clamp(120.0, 4000.0);
            let pa = PlotArea::from_ranges(
                (xmin_data, xmax_data),
                (ymin_data, ymax_data),
                w,
                h,
                Margins::default(),
                7,
                6,
            );
            let mut svg = svg_open(w, h);
            render_chrome(
                &mut svg,
                args.title.as_deref(),
                args.xlabel.as_deref(),
                args.ylabel.as_deref(),
                w,
                h,
                &pa.margins,
            );
            render_x_axis(
                &mut svg,
                &pa.x_ticks,
                pa.x_domain,
                (pa.left(), pa.right()),
                pa.bottom(),
                pa.top(),
            );
            render_y_axis(
                &mut svg,
                &pa.y_ticks,
                pa.y_domain,
                (pa.top(), pa.bottom()),
                pa.left(),
                pa.right(),
            );
            let up = args.up_color.as_deref().unwrap_or("#73bf69");
            let down = args.down_color.as_deref().unwrap_or("#e02f44");
            // Candle width: 60% of the gap between consecutive x positions.
            let bw = if args.candles.len() < 2 {
                (pa.right() - pa.left()) * 0.05
            } else {
                let avg_gap = (pa.x_domain.1 - pa.x_domain.0) / (args.candles.len().max(1) as f64);
                (avg_gap / (pa.x_domain.1 - pa.x_domain.0).max(f64::EPSILON)
                    * (pa.right() - pa.left()))
                    * 0.6
            };
            for c in &args.candles {
                let cx = pa.scale_x(c.x);
                let yh = pa.scale_y(c.high);
                let yl = pa.scale_y(c.low);
                let yo = pa.scale_y(c.open);
                let yc = pa.scale_y(c.close);
                let color = if c.close >= c.open { up } else { down };
                let (top, bot) = if yo <= yc { (yo, yc) } else { (yc, yo) };
                // Wick.
                let _ = writeln!(
                    svg,
                    "<line x1=\"{cx:.2}\" y1=\"{yh:.2}\" x2=\"{cx:.2}\" y2=\"{yl:.2}\" \
                     stroke=\"{color}\" stroke-width=\"1\"/>"
                );
                // Body.
                let _ = writeln!(
                    svg,
                    "<rect x=\"{x:.2}\" y=\"{top:.2}\" width=\"{bw:.2}\" height=\"{bh:.2}\" \
                     fill=\"{color}\" opacity=\"0.85\"/>",
                    x = cx - bw / 2.0,
                    bh = (bot - top).max(1.0)
                );
            }
            svg.push_str("</svg>");
            let (xmin, xmax) = pa.x_domain;
            let (ymin, ymax) = pa.y_domain;
            let desc = format!(
                "Candlestick{} · {} candles · price [{}, {}] · time [{}, {}]",
                title_suffix(args.title.as_deref()),
                args.candles.len(),
                fmt_tick(ymin),
                fmt_tick(ymax),
                fmt_tick(xmin),
                fmt_tick(xmax),
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Daily OHLC",
                args: r#"{"candles": [{"x": 1, "open": 100, "high": 105, "low": 99, "close": 103}, {"x": 2, "open": 103, "high": 108, "low": 102, "close": 107}, {"x": 3, "open": 107, "high": 109, "low": 100, "close": 101}], "title": "ACME"}"#,
                note: Some("Up candles green, down candles red, wicks span low-high."),
            },
            SkillExample {
                title: "Custom up/down colors",
                args: r##"{"candles": [{"x": 1, "open": 50, "high": 55, "low": 48, "close": 52}, {"x": 2, "open": 52, "high": 53, "low": 49, "close": 50}], "up_color": "#10b981", "down_color": "#ef4444"}"##,
                note: Some("Override the default green/red palette."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Financial / market price-action chart over a time window.",
            "Any per-interval open/high/low/close summary view.",
            "Show volatility along with direction at a glance.",
        ]
    }
}

// ---------------------------------------------------------------------------
// chart_sparkline — tiny inline trend, no chrome
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SparklineArgs {
    /// `[x, y]` points; the renderer auto-scales to fill the box.
    points: Vec<[f64; 2]>,
    /// Stroke color. Default `#5794f2`.
    #[serde(default)]
    color: Option<String>,
    /// Fill the area under the line at this opacity (0-1). Default 0.18.
    #[serde(default)]
    fill_opacity: Option<f64>,
    /// Width in user-space units. Default 240.
    #[serde(default)]
    width: Option<f64>,
    /// Height in user-space units. Default 60.
    #[serde(default)]
    height: Option<f64>,
}

pub struct ChartSparkline;
impl Skill for ChartSparkline {
    fn name(&self) -> &'static str {
        "chart_sparkline"
    }
    fn description(&self) -> &'static str {
        "Tiny inline trend line — no axes, no title, just the shape. The miniature chart Edward \
        Tufte popularized; Grafana uses it inside its Stat panel sparkline option and in \
        compact dashboard tiles. Pass `(x, y)` points; the renderer auto-scales to fill the box. \
        Useful in tables and tight UIs."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SparklineArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<SparklineArgs>()?;
            ensure_min_len(&args.points, 2, "points")?;
            let w = args.width.unwrap_or(240.0).clamp(40.0, 4000.0);
            let h = args.height.unwrap_or(60.0).clamp(20.0, 4000.0);
            let color = args.color.as_deref().unwrap_or("#5794f2");
            let fill_op = args.fill_opacity.unwrap_or(0.18).clamp(0.0, 1.0);
            let mut svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" \
                 preserveAspectRatio=\"xMidYMid meet\" role=\"img\">"
            );
            let pts: Vec<(f64, f64)> = args.points.iter().map(|p| (p[0], p[1])).collect();
            draw_sparkline(&mut svg, &pts, 2.0, 2.0, w - 4.0, h - 4.0, color, fill_op);
            svg.push_str("</svg>");
            let desc = format!(
                "Sparkline · {} points · {}×{}",
                args.points.len(),
                fmt_tick(w),
                fmt_tick(h),
            );
            Ok(svg_result(svg, desc))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Tiny trend",
                args: r#"{"points": [[0, 10], [1, 12], [2, 11], [3, 15], [4, 18], [5, 16]]}"#,
                note: Some("Returns a 240x60 SVG with default blue stroke and 0.18 fill opacity."),
            },
            SkillExample {
                title: "Inline-table cell sparkline",
                args: r##"{"points": [[0, 5], [1, 6], [2, 8], [3, 7]], "width": 80, "height": 24, "color": "#10b981", "fill_opacity": 0}"##,
                note: Some("Drop fill_opacity to 0 for pure line. Tight size for table cells."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Inline trend mark next to a numeric value (Stat panel style).",
            "Compact table-cell trendlines.",
            "Render a Tufte-style minimalist data shape.",
        ]
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Chart.js bar chart",
                args: r#"{"library": "chartjs", "config": {"type": "bar", "data": {"labels": ["A", "B", "C"], "datasets": [{"label": "n", "data": [10, 20, 15]}]}}, "title": "Counts"}"#,
                note: Some("Returns HTML that loads chart.js from CDN and renders the config."),
            },
            SkillExample {
                title: "Plotly scatter with layout",
                args: r#"{"library": "plotly", "config": {"data": [{"x": [1, 2, 3], "y": [2, 5, 1], "type": "scatter"}], "layout": {"title": "Demo"}}, "height": "320px"}"#,
                note: Some("Plotly config uses {data, layout} directly."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Need pan / zoom / hover tooltips in a client that renders HTML.",
            "Embed a chart inside a Jupyter or browser-based MCP client.",
            "Plotly-only chart types (3D, surface, sankey, treemap) not in the static SVG set.",
        ]
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
        Box::new(ChartStat),
        Box::new(ChartGauge),
        Box::new(ChartBarGauge),
        Box::new(ChartStateTimeline),
        Box::new(ChartCandlestick),
        Box::new(ChartSparkline),
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
