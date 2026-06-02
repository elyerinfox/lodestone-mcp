//! Specialist chart types — polar (antenna patterns), Smith (RF impedance),
//! waterfall (spectrogram heatmap), compass rose, sky plot (az/el dome),
//! density map (2-D histogram heatmap). Pure-Rust SVG, no deps beyond what
//! `chart.rs` already pulls in. Wrapped as MCP `image/svg+xml`.

use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, Content, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::invalid;
use crate::skills::{schema_for, Skill, SkillCtx};

// ---------------------------------------------------------------------------
// Tiny shared SVG helpers — independent of chart.rs internals so we don't
// have to expose `pub(crate)` fns across modules.
// ---------------------------------------------------------------------------

const W: f64 = 760.0;
const H: f64 = 760.0;

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

fn svg_result(svg: String, description: String) -> CallToolResult {
    let img = Content::image(b64(svg.as_bytes()), "image/svg+xml");
    let txt = Content::text(description);
    CallToolResult::success(vec![img, txt])
}

fn svg_open(w: f64, h: f64, title: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" \
         font-family=\"system-ui, sans-serif\" font-size=\"12\">\
         <rect width=\"{w}\" height=\"{h}\" fill=\"#ffffff\"/>\
         <text x=\"{cx}\" y=\"22\" text-anchor=\"middle\" font-size=\"16\" \
         font-weight=\"600\">{t}</text>",
        cx = w / 2.0,
        t = xml_escape(title),
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Map a value in `[lo, hi]` to a viridis-ish RGB. Cheap 5-stop interp.
fn viridis(t: f64) -> (u8, u8, u8) {
    let stops = [
        (0.0, (68, 1, 84)),
        (0.25, (59, 82, 139)),
        (0.5, (33, 144, 141)),
        (0.75, (94, 201, 98)),
        (1.0, (253, 231, 37)),
    ];
    let t = t.clamp(0.0, 1.0);
    for i in 0..stops.len() - 1 {
        let (a, ca) = stops[i];
        let (b, cb) = stops[i + 1];
        if t <= b {
            let u = (t - a) / (b - a).max(1e-12);
            return (
                (ca.0 as f64 + u * (cb.0 as f64 - ca.0 as f64)).round() as u8,
                (ca.1 as f64 + u * (cb.1 as f64 - ca.1 as f64)).round() as u8,
                (ca.2 as f64 + u * (cb.2 as f64 - ca.2 as f64)).round() as u8,
            );
        }
    }
    let (_, c) = stops[stops.len() - 1];
    (c.0, c.1, c.2)
}

// ---------------------------------------------------------------------------
// Polar plot — antenna pattern, magnitude vs angle.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PolarArgs {
    /// Sample magnitudes (linear). One per angle.
    magnitudes: Vec<f64>,
    /// Sample angles in degrees. Same length as magnitudes. Defaults to
    /// `0..360` evenly spaced if omitted.
    #[serde(default)]
    angles_deg: Option<Vec<f64>>,
    /// Render in dB (10·log10 of normalized magnitude). Default true — that
    /// matches how antenna patterns are conventionally shown.
    #[serde(default)]
    use_db: Option<bool>,
    /// Minimum dB on the radial axis (default -40 dB).
    #[serde(default)]
    db_min: Option<f64>,
    /// Optional title above the plot.
    #[serde(default)]
    title: Option<String>,
}

pub struct ChartPolar;
impl Skill for ChartPolar {
    fn name(&self) -> &'static str {
        "chart_polar"
    }
    fn description(&self) -> &'static str {
        "Polar magnitude-vs-angle plot — typical use is an antenna gain \
        pattern (dB by default, normalized so the peak sits at 0 dB). \
        Renders concentric dB rings, angle spokes every 30°, and a closed \
        curve through the samples."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PolarArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PolarArgs>()?;
            if a.magnitudes.is_empty() {
                return Err(invalid("magnitudes empty"));
            }
            let n = a.magnitudes.len();
            let angles: Vec<f64> = a
                .angles_deg
                .unwrap_or_else(|| (0..n).map(|i| 360.0 * i as f64 / n as f64).collect());
            if angles.len() != n {
                return Err(invalid("angles_deg length must match magnitudes"));
            }
            let use_db = a.use_db.unwrap_or(true);
            let db_min = a.db_min.unwrap_or(-40.0);
            let peak = a
                .magnitudes
                .iter()
                .cloned()
                .fold(0_f64, f64::max)
                .max(1e-12);
            let radials: Vec<f64> = if use_db {
                a.magnitudes
                    .iter()
                    .map(|m| 10.0 * (m / peak).max(1e-12).log10())
                    .collect()
            } else {
                a.magnitudes.iter().map(|m| m / peak).collect()
            };
            let (rmin, rmax) = if use_db { (db_min, 0.0) } else { (0.0, 1.0) };
            let title = a.title.clone().unwrap_or_else(|| "Polar pattern".into());
            let mut svg = svg_open(W, H, &title);
            let cx = W / 2.0;
            let cy = H / 2.0 + 10.0;
            let radius = (W.min(H) / 2.0) - 60.0;
            // Concentric rings + radial labels.
            let ring_count = 4;
            for i in 1..=ring_count {
                let r = radius * (i as f64 / ring_count as f64);
                let val = rmin + (rmax - rmin) * (i as f64 / ring_count as f64);
                let lbl = if use_db {
                    format!("{val:.0} dB")
                } else {
                    format!("{val:.2}")
                };
                let _ = write!(
                    svg,
                    "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" fill=\"none\" \
                     stroke=\"#dddddd\" stroke-width=\"1\"/>\
                     <text x=\"{tx}\" y=\"{ty}\" font-size=\"10\" fill=\"#666\" \
                     text-anchor=\"start\">{lbl}</text>",
                    tx = cx + 2.0,
                    ty = cy - r,
                );
            }
            // Angle spokes every 30°.
            for deg in (0..360).step_by(30) {
                let th = (deg as f64 - 90.0).to_radians();
                let x = cx + radius * th.cos();
                let y = cy + radius * th.sin();
                let _ = write!(
                    svg,
                    "<line x1=\"{cx}\" y1=\"{cy}\" x2=\"{x}\" y2=\"{y}\" \
                     stroke=\"#dddddd\" stroke-width=\"1\"/>\
                     <text x=\"{tx}\" y=\"{ty}\" font-size=\"10\" fill=\"#444\" \
                     text-anchor=\"middle\">{deg}°</text>",
                    tx = cx + (radius + 12.0) * th.cos(),
                    ty = cy + (radius + 12.0) * th.sin() + 4.0,
                );
            }
            // Curve.
            let mut path = String::new();
            for (i, (deg, r)) in angles.iter().zip(radials.iter()).enumerate() {
                let norm = ((r - rmin) / (rmax - rmin)).clamp(0.0, 1.0);
                let rr = radius * norm;
                let th = (deg - 90.0).to_radians();
                let x = cx + rr * th.cos();
                let y = cy + rr * th.sin();
                if i == 0 {
                    let _ = write!(path, "M{x:.2} {y:.2} ");
                } else {
                    let _ = write!(path, "L{x:.2} {y:.2} ");
                }
            }
            path.push('Z');
            let _ = write!(
                svg,
                "<path d=\"{path}\" fill=\"#3b82f6\" fill-opacity=\"0.18\" \
                 stroke=\"#1d4ed8\" stroke-width=\"1.5\"/>"
            );
            svg.push_str("</svg>");
            let desc = format!("{title}: {n} samples, peak normalized.");
            Ok(svg_result(svg, desc))
        })
    }
}

// ---------------------------------------------------------------------------
// Smith chart — normalized impedance, constant-resistance + reactance arcs.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SmithArgs {
    /// Complex impedances `[r, x]` (Ω). Will be normalized by `z0`.
    impedances: Vec<[f64; 2]>,
    /// Reference impedance (Ω). Default 50.
    #[serde(default)]
    z0: Option<f64>,
    /// Optional label per point.
    #[serde(default)]
    labels: Option<Vec<String>>,
    /// Optional title above the chart.
    #[serde(default)]
    title: Option<String>,
}

pub struct ChartSmith;
impl Skill for ChartSmith {
    fn name(&self) -> &'static str {
        "chart_smith"
    }
    fn description(&self) -> &'static str {
        "Smith chart — plot normalized impedances on the standard Γ-plane. \
        Draws the outer reflection circle, constant-resistance circles at \
        r = {0, 0.5, 1, 2, 5}, and constant-reactance arcs at x = ±{0.5, \
        1, 2, 5}, then marks each impedance with a dot (optionally labeled)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SmithArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SmithArgs>()?;
            let z0 = a.z0.unwrap_or(50.0).max(1e-9);
            let title = a.title.clone().unwrap_or_else(|| "Smith chart".into());
            let mut svg = svg_open(W, H, &title);
            let cx = W / 2.0;
            let cy = H / 2.0 + 10.0;
            let r0 = (W.min(H) / 2.0) - 60.0;
            // Outer unit circle (Γ-disc edge).
            let _ = write!(
                svg,
                "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r0}\" fill=\"#ffffff\" \
                 stroke=\"#222\" stroke-width=\"1.5\"/>"
            );
            // Constant-resistance circles in Γ-plane: center (r/(1+r), 0), radius 1/(1+r).
            for &r in &[0.0, 0.5, 1.0, 2.0, 5.0] {
                let denom = 1.0 + r;
                let cgx = r / denom;
                let rad = 1.0 / denom;
                let _ = write!(
                    svg,
                    "<circle cx=\"{xc}\" cy=\"{cy}\" r=\"{rd}\" fill=\"none\" \
                     stroke=\"#bbb\" stroke-width=\"1\"/>",
                    xc = cx + r0 * cgx,
                    rd = r0 * rad,
                );
            }
            // Constant-reactance arcs: center (1, 1/x), radius 1/|x|. Clip to outer disc.
            for &x in &[0.5_f64, 1.0, 2.0, 5.0] {
                for sign in [1.0, -1.0] {
                    let xx = sign * x;
                    let cgx = 1.0;
                    let cgy = 1.0 / xx;
                    let rad = 1.0 / xx.abs();
                    let _ = write!(
                        svg,
                        "<circle cx=\"{xc}\" cy=\"{yc}\" r=\"{rd}\" fill=\"none\" \
                         stroke=\"#bbb\" stroke-width=\"1\" \
                         clip-path=\"circle({r0}px at {cx}px {cy}px)\"/>",
                        xc = cx + r0 * cgx,
                        yc = cy - r0 * cgy,
                        rd = r0 * rad,
                    );
                }
            }
            // Horizontal real-axis baseline.
            let _ = write!(
                svg,
                "<line x1=\"{x1}\" y1=\"{cy}\" x2=\"{x2}\" y2=\"{cy}\" \
                 stroke=\"#aaa\" stroke-width=\"1\"/>",
                x1 = cx - r0,
                x2 = cx + r0,
            );
            // Plot impedances.
            for (i, z) in a.impedances.iter().enumerate() {
                let zr = z[0] / z0;
                let zi = z[1] / z0;
                let denom = (zr + 1.0).powi(2) + zi.powi(2);
                let gr = ((zr * zr + zi * zi) - 1.0) / denom;
                let gi = (2.0 * zi) / denom;
                let px = cx + r0 * gr;
                let py = cy - r0 * gi;
                let _ = write!(
                    svg,
                    "<circle cx=\"{px}\" cy=\"{py}\" r=\"4\" fill=\"#dc2626\"/>"
                );
                if let Some(labels) = a.labels.as_ref() {
                    if let Some(l) = labels.get(i) {
                        let _ = write!(
                            svg,
                            "<text x=\"{lx}\" y=\"{ly}\" font-size=\"10\" \
                             fill=\"#7f1d1d\">{txt}</text>",
                            lx = px + 6.0,
                            ly = py - 6.0,
                            txt = xml_escape(l),
                        );
                    }
                }
            }
            svg.push_str("</svg>");
            Ok(svg_result(
                svg,
                format!("{title}: {} impedances, Z0={z0:.0} Ω.", a.impedances.len()),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Waterfall — frequency × time × power heatmap.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaterfallArgs {
    /// Power matrix: outer rows are time bins (top = oldest), inner are freq bins.
    power: Vec<Vec<f64>>,
    /// Optional power-dB clip range. Defaults: auto from the data.
    #[serde(default)]
    db_min: Option<f64>,
    /// Optional upper power-dB clip value (defaults to data max).
    #[serde(default)]
    db_max: Option<f64>,
    /// Frequency-axis label.
    #[serde(default)]
    freq_label: Option<String>,
    /// Optional title above the chart.
    #[serde(default)]
    title: Option<String>,
}

pub struct ChartWaterfall;
impl Skill for ChartWaterfall {
    fn name(&self) -> &'static str {
        "chart_waterfall"
    }
    fn description(&self) -> &'static str {
        "Waterfall (spectrogram) heatmap — render a 2-D power matrix as a \
        viridis-colored grid, time on the Y axis (top = oldest), frequency \
        on the X axis. Auto-clips to the data min/max in dB unless an \
        explicit range is provided."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WaterfallArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<WaterfallArgs>()?;
            if a.power.is_empty() || a.power[0].is_empty() {
                return Err(invalid("power matrix empty"));
            }
            let n_rows = a.power.len();
            let n_cols = a.power[0].len();
            for r in &a.power {
                if r.len() != n_cols {
                    return Err(invalid("power matrix ragged"));
                }
            }
            let (mut dmin, mut dmax) = (f64::INFINITY, f64::NEG_INFINITY);
            for r in &a.power {
                for v in r {
                    if v.is_finite() {
                        dmin = dmin.min(*v);
                        dmax = dmax.max(*v);
                    }
                }
            }
            let lo = a.db_min.unwrap_or(dmin);
            let hi = a.db_max.unwrap_or(dmax);
            let span = (hi - lo).max(1e-12);
            let title = a.title.clone().unwrap_or_else(|| "Waterfall".into());
            let mut svg = svg_open(W, H, &title);
            let plot_x = 70.0;
            let plot_y = 50.0;
            let plot_w = W - 120.0;
            let plot_h = H - 100.0;
            let cell_w = plot_w / n_cols as f64;
            let cell_h = plot_h / n_rows as f64;
            for (ri, row) in a.power.iter().enumerate() {
                for (ci, v) in row.iter().enumerate() {
                    let t = ((v - lo) / span).clamp(0.0, 1.0);
                    let (r, g, b) = viridis(t);
                    let _ = write!(
                        svg,
                        "<rect x=\"{x:.2}\" y=\"{y:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" \
                         fill=\"#{r:02x}{g:02x}{b:02x}\"/>",
                        x = plot_x + ci as f64 * cell_w,
                        y = plot_y + ri as f64 * cell_h,
                        w = cell_w + 0.5,
                        h = cell_h + 0.5,
                    );
                }
            }
            // Axis frame.
            let _ = write!(
                svg,
                "<rect x=\"{plot_x}\" y=\"{plot_y}\" width=\"{plot_w}\" \
                 height=\"{plot_h}\" fill=\"none\" stroke=\"#222\" stroke-width=\"1\"/>"
            );
            let freq_label = a
                .freq_label
                .clone()
                .unwrap_or_else(|| "frequency bin".into());
            let _ = write!(
                svg,
                "<text x=\"{lx}\" y=\"{ly}\" text-anchor=\"middle\" \
                 font-size=\"12\">{lbl}</text>\
                 <text x=\"20\" y=\"{tly}\" text-anchor=\"middle\" font-size=\"12\" \
                 transform=\"rotate(-90 20 {tly})\">time bin (top=oldest)</text>",
                lx = plot_x + plot_w / 2.0,
                ly = plot_y + plot_h + 30.0,
                lbl = xml_escape(&freq_label),
                tly = plot_y + plot_h / 2.0,
            );
            // Color-bar.
            let bar_x = plot_x + plot_w + 20.0;
            let bar_w = 12.0;
            let bar_h = plot_h;
            let steps = 64;
            for i in 0..steps {
                let t = i as f64 / (steps - 1) as f64;
                let (r, g, b) = viridis(1.0 - t);
                let _ = write!(
                    svg,
                    "<rect x=\"{bar_x}\" y=\"{y:.2}\" width=\"{bar_w}\" \
                     height=\"{h:.2}\" fill=\"#{r:02x}{g:02x}{b:02x}\"/>",
                    y = plot_y + t * bar_h,
                    h = bar_h / steps as f64 + 0.5,
                );
            }
            let _ = write!(
                svg,
                "<text x=\"{xt}\" y=\"{yt}\" font-size=\"10\" fill=\"#222\">{hi:.1}</text>\
                 <text x=\"{xt}\" y=\"{yb}\" font-size=\"10\" fill=\"#222\">{lo:.1}</text>",
                xt = bar_x + bar_w + 4.0,
                yt = plot_y + 8.0,
                yb = plot_y + bar_h,
            );
            svg.push_str("</svg>");
            Ok(svg_result(
                svg,
                format!("{title}: {n_rows}×{n_cols} bins, range [{lo:.1}, {hi:.1}] dB."),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Compass rose — bearings + magnitudes (wind rose style).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CompassArgs {
    /// One sample per bearing slice. 16 slices (one every 22.5°) is conventional.
    magnitudes_by_bearing: Vec<f64>,
    /// Optional title above the rose.
    #[serde(default)]
    title: Option<String>,
}

pub struct ChartCompass;
impl Skill for ChartCompass {
    fn name(&self) -> &'static str {
        "chart_compass_rose"
    }
    fn description(&self) -> &'static str {
        "Compass rose / wind rose — render magnitude vs bearing as radial \
        bars around a compass with N/E/S/W markers. Useful for wind direction \
        frequency, direction-of-arrival histograms, or any bearing-binned signal."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CompassArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CompassArgs>()?;
            if a.magnitudes_by_bearing.is_empty() {
                return Err(invalid("magnitudes_by_bearing empty"));
            }
            let n = a.magnitudes_by_bearing.len();
            let title = a.title.clone().unwrap_or_else(|| "Compass rose".into());
            let mut svg = svg_open(W, H, &title);
            let cx = W / 2.0;
            let cy = H / 2.0 + 10.0;
            let radius = (W.min(H) / 2.0) - 70.0;
            let peak = a
                .magnitudes_by_bearing
                .iter()
                .cloned()
                .fold(0_f64, f64::max)
                .max(1e-12);
            // Background rings.
            for i in 1..=4 {
                let r = radius * (i as f64 / 4.0);
                let _ = write!(
                    svg,
                    "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" fill=\"none\" \
                     stroke=\"#e5e7eb\" stroke-width=\"1\"/>"
                );
            }
            // Bars — wedges.
            let slice = std::f64::consts::TAU / n as f64;
            for (i, m) in a.magnitudes_by_bearing.iter().enumerate() {
                let rr = radius * (m / peak);
                let bearing = i as f64 * slice;
                let half = slice * 0.45;
                let a0 = bearing - half - std::f64::consts::FRAC_PI_2;
                let a1 = bearing + half - std::f64::consts::FRAC_PI_2;
                let x0 = cx + rr * a0.cos();
                let y0 = cy + rr * a0.sin();
                let x1 = cx + rr * a1.cos();
                let y1 = cy + rr * a1.sin();
                let _ = write!(
                    svg,
                    "<path d=\"M{cx:.2} {cy:.2} L{x0:.2} {y0:.2} \
                     A{rr:.2} {rr:.2} 0 0 1 {x1:.2} {y1:.2} Z\" \
                     fill=\"#3b82f6\" fill-opacity=\"0.55\" stroke=\"#1d4ed8\" \
                     stroke-width=\"1\"/>"
                );
            }
            // Cardinal markers.
            for (deg, label) in [(0, "N"), (90, "E"), (180, "S"), (270, "W")] {
                let th = (deg as f64 - 90.0).to_radians();
                let tx = cx + (radius + 18.0) * th.cos();
                let ty = cy + (radius + 18.0) * th.sin() + 4.0;
                let _ = write!(
                    svg,
                    "<text x=\"{tx}\" y=\"{ty}\" text-anchor=\"middle\" \
                     font-size=\"14\" font-weight=\"600\">{label}</text>"
                );
            }
            svg.push_str("</svg>");
            Ok(svg_result(
                svg,
                format!("{title}: {n} bearing slices, peak {peak:.3}."),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Sky plot — azimuth (compass) × elevation (radial, 90° at center).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SkyArgs {
    /// One entry per object: `[az_deg, el_deg]`.
    az_el: Vec<[f64; 2]>,
    /// Optional label per point (e.g. satellite PRN).
    #[serde(default)]
    labels: Option<Vec<String>>,
    /// Optional title above the chart.
    #[serde(default)]
    title: Option<String>,
}

pub struct ChartSkyplot;
impl Skill for ChartSkyplot {
    fn name(&self) -> &'static str {
        "chart_skyplot"
    }
    fn description(&self) -> &'static str {
        "Sky plot — overhead dome showing object positions in (azimuth, \
        elevation) coordinates. Zenith (el=90°) is at the center; the \
        horizon (el=0°) is the outer ring. Elevation rings at 30°/60° and \
        cardinal azimuth markers are drawn for reference."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SkyArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SkyArgs>()?;
            let title = a.title.clone().unwrap_or_else(|| "Sky plot".into());
            let mut svg = svg_open(W, H, &title);
            let cx = W / 2.0;
            let cy = H / 2.0 + 10.0;
            let radius = (W.min(H) / 2.0) - 70.0;
            // Elevation rings: 0° (outer), 30°, 60°.
            for &el in &[0.0_f64, 30.0, 60.0] {
                let r = radius * ((90.0 - el) / 90.0);
                let _ = write!(
                    svg,
                    "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"{r}\" fill=\"none\" \
                     stroke=\"#cbd5e1\" stroke-width=\"1\"/>\
                     <text x=\"{tx}\" y=\"{ty}\" font-size=\"10\" fill=\"#475569\">{el}°</text>",
                    tx = cx + 4.0,
                    ty = cy - r + 12.0,
                );
            }
            // Cardinal azimuth markers.
            for (deg, label) in [(0, "N"), (90, "E"), (180, "S"), (270, "W")] {
                let th = (deg as f64 - 90.0).to_radians();
                let x = cx + (radius + 18.0) * th.cos();
                let y = cy + (radius + 18.0) * th.sin() + 4.0;
                let _ = write!(
                    svg,
                    "<line x1=\"{cx}\" y1=\"{cy}\" x2=\"{ex}\" y2=\"{ey}\" \
                     stroke=\"#cbd5e1\" stroke-width=\"1\"/>\
                     <text x=\"{x}\" y=\"{y}\" text-anchor=\"middle\" font-size=\"14\" \
                     font-weight=\"600\">{label}</text>",
                    ex = cx + radius * th.cos(),
                    ey = cy + radius * th.sin(),
                );
            }
            // Points.
            for (i, p) in a.az_el.iter().enumerate() {
                let az = p[0];
                let el = p[1].clamp(0.0, 90.0);
                let r = radius * ((90.0 - el) / 90.0);
                let th = (az - 90.0).to_radians();
                let x = cx + r * th.cos();
                let y = cy + r * th.sin();
                let _ = write!(
                    svg,
                    "<circle cx=\"{x:.2}\" cy=\"{y:.2}\" r=\"5\" fill=\"#ef4444\" \
                     stroke=\"#7f1d1d\" stroke-width=\"1\"/>"
                );
                if let Some(labels) = a.labels.as_ref() {
                    if let Some(l) = labels.get(i) {
                        let _ = write!(
                            svg,
                            "<text x=\"{lx:.2}\" y=\"{ly:.2}\" font-size=\"10\" \
                             fill=\"#7f1d1d\">{t}</text>",
                            lx = x + 7.0,
                            ly = y - 5.0,
                            t = xml_escape(l),
                        );
                    }
                }
            }
            svg.push_str("</svg>");
            Ok(svg_result(
                svg,
                format!("{title}: {} sky objects plotted.", a.az_el.len()),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Density map — 2-D histogram heatmap.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DensityArgs {
    /// XY samples to bin.
    points: Vec<[f64; 2]>,
    /// Bin counts along X / Y. Default 32 × 32.
    #[serde(default)]
    nx: Option<usize>,
    /// Bin count along Y (default 32).
    #[serde(default)]
    ny: Option<usize>,
    /// Optional title above the chart.
    #[serde(default)]
    title: Option<String>,
}

pub struct ChartDensityMap;
impl Skill for ChartDensityMap {
    fn name(&self) -> &'static str {
        "chart_density_map"
    }
    fn description(&self) -> &'static str {
        "2-D density / heatmap from raw point samples — bins the points \
        into an nx × ny grid (default 32 × 32) and renders the counts as a \
        viridis-colored grid with a colorbar. Use for scatter that's too \
        dense to read as dots, or for spatial-distribution overviews."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DensityArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DensityArgs>()?;
            if a.points.is_empty() {
                return Err(invalid("points empty"));
            }
            let nx = a.nx.unwrap_or(32).max(2);
            let ny = a.ny.unwrap_or(32).max(2);
            let (mut xmn, mut xmx) = (f64::INFINITY, f64::NEG_INFINITY);
            let (mut ymn, mut ymx) = (f64::INFINITY, f64::NEG_INFINITY);
            for p in &a.points {
                xmn = xmn.min(p[0]);
                xmx = xmx.max(p[0]);
                ymn = ymn.min(p[1]);
                ymx = ymx.max(p[1]);
            }
            let dx = (xmx - xmn).max(1e-12);
            let dy = (ymx - ymn).max(1e-12);
            let mut bins = vec![vec![0_u32; nx]; ny];
            for p in &a.points {
                let ix = (((p[0] - xmn) / dx) * nx as f64).floor() as i64;
                let iy = (((p[1] - ymn) / dy) * ny as f64).floor() as i64;
                let ix = ix.clamp(0, nx as i64 - 1) as usize;
                let iy = iy.clamp(0, ny as i64 - 1) as usize;
                bins[iy][ix] += 1;
            }
            let max_c = bins.iter().flatten().copied().max().unwrap_or(0).max(1) as f64;
            let title = a.title.clone().unwrap_or_else(|| "Density map".into());
            let mut svg = svg_open(W, H, &title);
            let plot_x = 70.0;
            let plot_y = 50.0;
            let plot_w = W - 130.0;
            let plot_h = H - 100.0;
            let cell_w = plot_w / nx as f64;
            let cell_h = plot_h / ny as f64;
            for (iy, row) in bins.iter().enumerate() {
                for (ix, c) in row.iter().enumerate() {
                    let t = (*c as f64) / max_c;
                    let (r, g, b) = viridis(t);
                    let _ = write!(
                        svg,
                        "<rect x=\"{x:.2}\" y=\"{y:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" \
                         fill=\"#{r:02x}{g:02x}{b:02x}\"/>",
                        x = plot_x + ix as f64 * cell_w,
                        // Flip Y so larger y values are at the top.
                        y = plot_y + plot_h - (iy as f64 + 1.0) * cell_h,
                        w = cell_w + 0.5,
                        h = cell_h + 0.5,
                    );
                }
            }
            let _ = write!(
                svg,
                "<rect x=\"{plot_x}\" y=\"{plot_y}\" width=\"{plot_w}\" \
                 height=\"{plot_h}\" fill=\"none\" stroke=\"#222\" stroke-width=\"1\"/>"
            );
            // Axis labels (min/max at corners).
            let _ = write!(
                svg,
                "<text x=\"{xt0}\" y=\"{yt0}\" font-size=\"10\">{xmn:.2}</text>\
                 <text x=\"{xt1}\" y=\"{yt0}\" text-anchor=\"end\" font-size=\"10\">{xmx:.2}</text>\
                 <text x=\"{xtl}\" y=\"{ytl1}\" text-anchor=\"end\" font-size=\"10\">{ymn:.2}</text>\
                 <text x=\"{xtl}\" y=\"{ytl0}\" text-anchor=\"end\" font-size=\"10\">{ymx:.2}</text>",
                xt0 = plot_x,
                xt1 = plot_x + plot_w,
                yt0 = plot_y + plot_h + 14.0,
                xtl = plot_x - 6.0,
                ytl0 = plot_y + 8.0,
                ytl1 = plot_y + plot_h,
            );
            // Colorbar.
            let bar_x = plot_x + plot_w + 20.0;
            let bar_w = 12.0;
            let bar_h = plot_h;
            let steps = 64;
            for i in 0..steps {
                let t = i as f64 / (steps - 1) as f64;
                let (r, g, b) = viridis(1.0 - t);
                let _ = write!(
                    svg,
                    "<rect x=\"{bar_x}\" y=\"{y:.2}\" width=\"{bar_w}\" \
                     height=\"{h:.2}\" fill=\"#{r:02x}{g:02x}{b:02x}\"/>",
                    y = plot_y + t * bar_h,
                    h = bar_h / steps as f64 + 0.5,
                );
            }
            let _ = write!(
                svg,
                "<text x=\"{xt}\" y=\"{yt}\" font-size=\"10\">{maxc}</text>\
                 <text x=\"{xt}\" y=\"{yb}\" font-size=\"10\">0</text>",
                xt = bar_x + bar_w + 4.0,
                yt = plot_y + 8.0,
                yb = plot_y + bar_h,
                maxc = max_c as u32,
            );
            svg.push_str("</svg>");
            Ok(svg_result(
                svg,
                format!(
                    "{title}: {} points binned into {nx}×{ny}, peak count {peak}.",
                    a.points.len(),
                    peak = max_c as u32,
                ),
            ))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(ChartPolar),
        Box::new(ChartSmith),
        Box::new(ChartWaterfall),
        Box::new(ChartCompass),
        Box::new(ChartSkyplot),
        Box::new(ChartDensityMap),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viridis_stops_clamped() {
        let (r, _, _) = viridis(0.0);
        assert_eq!(r, 68);
        let (r, _, _) = viridis(1.0);
        assert_eq!(r, 253);
        let (r, _, _) = viridis(-1.0);
        assert_eq!(r, 68);
        let (r, _, _) = viridis(2.0);
        assert_eq!(r, 253);
    }
}
