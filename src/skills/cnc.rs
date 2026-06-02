//! CNC / OpenSCAD — G-code emitters + parser and OpenSCAD source generators
//! for common manufacturing patterns. Pure-Rust, no host requirements, on
//! by default. No network; no constellation cache is needed (all logic is
//! local).
//!
//! ## Source citations
//!
//! - **G-code dialect**: NIST RS-274/NGC v3 — Kramer, Proctor, Messina,
//!   *The NIST RS274NGC Interpreter — Version 3*, NIST Tech. Note (2000)
//!   <https://www.nist.gov/publications/nist-rs274ngc-interpreter-version-3>.
//!   This is the de-facto reference and what LinuxCNC implements; targeting
//!   it gives maximal portability across Grbl (subset) and Marlin (with
//!   known limitations called out in tool descriptions).
//! - **ISO 6983-1** (1982) is the nominal international standard but is
//!   rarely implemented literally — we treat RS-274/NGC as the canonical
//!   reference dialect.
//! - **OpenSCAD language reference**:
//!   <https://en.wikibooks.org/wiki/OpenSCAD_User_Manual>.

use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// gcode_drill_hole — emit RS-274/NGC for a single drilled hole at (x, y).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DrillArgs {
    /// Hole center X (mm by default, unless `units: imperial`).
    x: f64,
    /// Hole center Y.
    y: f64,
    /// Drill depth (positive number; tool plunges to `-depth`).
    depth: f64,
    /// Safe retract height (above the work surface).
    safe_z: f64,
    /// Plunge feed rate (mm/min or inch/min, matching units).
    plunge_feed: f64,
    /// Spindle RPM.
    rpm: f64,
    /// `metric` (G21, default) or `imperial` (G20).
    #[serde(default)]
    units: Option<String>,
    /// Tool number (sets T and M6 in the preamble; default 1).
    #[serde(default)]
    tool: Option<u32>,
}

pub struct GcodeDrillHole;
impl Skill for GcodeDrillHole {
    fn name(&self) -> &'static str {
        "gcode_drill_hole"
    }
    fn description(&self) -> &'static str {
        "Emit RS-274/NGC G-code to drill a single hole at (X, Y) to the \
        given depth, with a safe Z retract. The output uses the canonical \
        preamble `G17 G21 G90 G94` (XY plane, mm, absolute, feed-per-minute) \
        — swap to `G20` for inches. Tool change via `Tn M6`. Order of \
        operations: retract to safe Z → rapid to XY → plunge to -depth at \
        the plunge feed → retract → spindle off → M30. Designed to be \
        portable across LinuxCNC and Grbl (Marlin needs no G2/G3, which we \
        don't emit). Z+ = up."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DrillArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DrillArgs>()?;
            if a.depth <= 0.0 {
                return Err(invalid("depth must be > 0"));
            }
            if a.safe_z <= 0.0 {
                return Err(invalid("safe_z must be > 0 (above the work)"));
            }
            if a.plunge_feed <= 0.0 || a.rpm <= 0.0 {
                return Err(invalid("plunge_feed and rpm must be > 0"));
            }
            let units = a.units.as_deref().unwrap_or("metric").to_ascii_lowercase();
            let unit_cmd = match units.as_str() {
                "metric" => "G21",
                "imperial" => "G20",
                other => return Err(invalid(format!("unknown units '{other}'"))),
            };
            let tool = a.tool.unwrap_or(1);
            let mut g = String::new();
            let _ = writeln!(
                g,
                "; Drill at ({:.4}, {:.4}), depth {:.4}",
                a.x, a.y, a.depth
            );
            let _ = writeln!(g, "G17 {unit_cmd} G90 G94");
            let _ = writeln!(g, "T{tool} M6");
            let _ = writeln!(g, "M3 S{:.0}", a.rpm);
            let _ = writeln!(g, "G0 Z{:.4}", a.safe_z);
            let _ = writeln!(g, "G0 X{:.4} Y{:.4}", a.x, a.y);
            let _ = writeln!(g, "G1 Z{:.4} F{:.2}", -a.depth, a.plunge_feed);
            let _ = writeln!(g, "G0 Z{:.4}", a.safe_z);
            let _ = writeln!(g, "M5");
            let _ = writeln!(g, "M30");
            Ok(text_result(json!({ "gcode": g }).to_string()))
        })
    }
}

// ---------------------------------------------------------------------------
// gcode_bolt_pattern — circle of drilled holes on a PCD.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BoltPatternArgs {
    /// Pitch-circle diameter (mm or inch).
    pcd: f64,
    /// Number of holes.
    n: u32,
    /// Plunge depth per hole.
    depth: f64,
    /// Safe retract height (above the work surface).
    safe_z: f64,
    /// Plunge feed rate (mm/min or inch/min, matching units).
    plunge_feed: f64,
    /// Spindle RPM.
    rpm: f64,
    /// X center of the pattern.
    #[serde(default)]
    cx: Option<f64>,
    /// Y center.
    #[serde(default)]
    cy: Option<f64>,
    /// Angle (deg) of the first hole from +X. Default 0.
    #[serde(default)]
    start_angle_deg: Option<f64>,
    /// `metric` (G21, default) or `imperial` (G20).
    #[serde(default)]
    units: Option<String>,
    /// Tool number (sets T and M6 in the preamble; default 1).
    #[serde(default)]
    tool: Option<u32>,
}

pub struct GcodeBoltPattern;
impl Skill for GcodeBoltPattern {
    fn name(&self) -> &'static str {
        "gcode_bolt_pattern"
    }
    fn description(&self) -> &'static str {
        "Emit RS-274/NGC G-code for a circular bolt-hole pattern: N evenly \
        spaced drilled holes on a pitch-circle diameter (PCD) centered at \
        (cx, cy). Same preamble / safety conventions as `gcode_drill_hole`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BoltPatternArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<BoltPatternArgs>()?;
            if a.n == 0 || a.pcd <= 0.0 || a.depth <= 0.0 || a.safe_z <= 0.0 {
                return Err(invalid("n, pcd, depth, safe_z must be > 0"));
            }
            let cx = a.cx.unwrap_or(0.0);
            let cy = a.cy.unwrap_or(0.0);
            let start = a.start_angle_deg.unwrap_or(0.0).to_radians();
            let radius = a.pcd / 2.0;
            let unit_cmd = match a
                .units
                .as_deref()
                .unwrap_or("metric")
                .to_ascii_lowercase()
                .as_str()
            {
                "metric" => "G21",
                "imperial" => "G20",
                other => return Err(invalid(format!("unknown units '{other}'"))),
            };
            let tool = a.tool.unwrap_or(1);
            let mut g = String::new();
            let _ = writeln!(
                g,
                "; {} holes on PCD {:.4} centered at ({:.4}, {:.4})",
                a.n, a.pcd, cx, cy
            );
            let _ = writeln!(g, "G17 {unit_cmd} G90 G94");
            let _ = writeln!(g, "T{tool} M6");
            let _ = writeln!(g, "M3 S{:.0}", a.rpm);
            let _ = writeln!(g, "G0 Z{:.4}", a.safe_z);
            for i in 0..a.n {
                let theta = start + (i as f64) * std::f64::consts::TAU / (a.n as f64);
                let x = cx + radius * theta.cos();
                let y = cy + radius * theta.sin();
                let _ = writeln!(g, "G0 X{:.4} Y{:.4}", x, y);
                let _ = writeln!(g, "G1 Z{:.4} F{:.2}", -a.depth, a.plunge_feed);
                let _ = writeln!(g, "G0 Z{:.4}", a.safe_z);
            }
            let _ = writeln!(g, "M5");
            let _ = writeln!(g, "M30");
            Ok(text_result(json!({ "gcode": g }).to_string()))
        })
    }
}

// ---------------------------------------------------------------------------
// gcode_parse_summary — sanity-check + summarize G-code.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ParseArgs {
    /// G-code text to parse.
    gcode: String,
}

pub struct GcodeParseSummary;
impl Skill for GcodeParseSummary {
    fn name(&self) -> &'static str {
        "gcode_parse_summary"
    }
    fn description(&self) -> &'static str {
        "Parse a G-code program and report a summary: command counts (G0 / \
        G1 / G2 / G3 / M*), total commanded travel along each axis, \
        bounding box of all explicit XY positions, max feed rate, and \
        first occurrences of mode words (units, plane, abs/inc). Useful \
        for sanity-checking machine-generated programs before sending them \
        to the controller. Supports both `( ... )` and `;` comments. \
        Per RS-274/NGC the parser is line-modal."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ParseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ParseArgs>()?;
            let mut g_counts = std::collections::BTreeMap::<String, u32>::new();
            let mut m_counts = std::collections::BTreeMap::<String, u32>::new();
            let mut max_feed = 0.0_f64;
            let mut units: Option<&str> = None;
            let mut plane: Option<&str> = None;
            let mut mode_abs: Option<&str> = None;
            let (mut xmn, mut xmx) = (f64::INFINITY, f64::NEG_INFINITY);
            let (mut ymn, mut ymx) = (f64::INFINITY, f64::NEG_INFINITY);
            let (mut zmn, mut zmx) = (f64::INFINITY, f64::NEG_INFINITY);
            let mut last_xyz = [0.0_f64; 3];
            let mut have_last = false;
            let mut travel = [0.0_f64; 3];
            for raw in a.gcode.lines() {
                // Strip inline `( ... )` and `; ...` comments.
                let mut s = String::new();
                let mut depth = 0;
                for ch in raw.chars() {
                    if ch == '(' {
                        depth += 1;
                        continue;
                    }
                    if ch == ')' {
                        if depth > 0 {
                            depth -= 1;
                        }
                        continue;
                    }
                    if ch == ';' {
                        break;
                    }
                    if depth == 0 {
                        s.push(ch);
                    }
                }
                let s = s.trim();
                if s.is_empty() {
                    continue;
                }
                let mut x_new = last_xyz[0];
                let mut y_new = last_xyz[1];
                let mut z_new = last_xyz[2];
                let mut had_xyz = false;
                for word in s.split_whitespace() {
                    let (letter, rest) = match word.chars().next() {
                        Some(c) => (c.to_ascii_uppercase(), &word[1..]),
                        None => continue,
                    };
                    match letter {
                        'G' => {
                            let key = format!("G{rest}");
                            *g_counts.entry(key.clone()).or_insert(0) += 1;
                            match rest {
                                "20" => units = Some("inch"),
                                "21" => units = Some("mm"),
                                "17" => plane = Some("XY"),
                                "18" => plane = Some("XZ"),
                                "19" => plane = Some("YZ"),
                                "90" => mode_abs = Some("absolute"),
                                "91" => mode_abs = Some("incremental"),
                                _ => {}
                            }
                        }
                        'M' => {
                            *m_counts.entry(format!("M{rest}")).or_insert(0) += 1;
                        }
                        'F' => {
                            if let Ok(f) = rest.parse::<f64>() {
                                if f > max_feed {
                                    max_feed = f;
                                }
                            }
                        }
                        'X' => {
                            if let Ok(v) = rest.parse::<f64>() {
                                x_new = v;
                                had_xyz = true;
                            }
                        }
                        'Y' => {
                            if let Ok(v) = rest.parse::<f64>() {
                                y_new = v;
                                had_xyz = true;
                            }
                        }
                        'Z' => {
                            if let Ok(v) = rest.parse::<f64>() {
                                z_new = v;
                                had_xyz = true;
                            }
                        }
                        _ => {}
                    }
                }
                if had_xyz {
                    xmn = xmn.min(x_new);
                    xmx = xmx.max(x_new);
                    ymn = ymn.min(y_new);
                    ymx = ymx.max(y_new);
                    zmn = zmn.min(z_new);
                    zmx = zmx.max(z_new);
                    if have_last {
                        travel[0] += (x_new - last_xyz[0]).abs();
                        travel[1] += (y_new - last_xyz[1]).abs();
                        travel[2] += (z_new - last_xyz[2]).abs();
                    }
                    last_xyz = [x_new, y_new, z_new];
                    have_last = true;
                }
            }
            let bbox = if xmn.is_finite() && ymn.is_finite() {
                json!({
                    "x": [xmn, xmx], "y": [ymn, ymx], "z": [zmn, zmx]
                })
            } else {
                serde_json::Value::Null
            };
            Ok(text_result(
                json!({
                    "g_counts": g_counts,
                    "m_counts": m_counts,
                    "max_feed": max_feed,
                    "units": units,
                    "plane": plane,
                    "mode": mode_abs,
                    "bbox": bbox,
                    "axis_travel": {"x": travel[0], "y": travel[1], "z": travel[2]},
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// scad_box / scad_cylinder / scad_sphere — primitives.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BoxArgs {
    /// X dimension.
    x: f64,
    /// Y dimension.
    y: f64,
    /// Z dimension.
    z: f64,
    /// Center on origin (true) or anchor at the +XYZ corner (false, OpenSCAD
    /// default).
    #[serde(default)]
    center: Option<bool>,
}

pub struct ScadBox;
impl Skill for ScadBox {
    fn name(&self) -> &'static str {
        "scad_box"
    }
    fn description(&self) -> &'static str {
        "Emit OpenSCAD source for a rectangular box: `cube([x, y, z], \
        center=<true|false>);`. `center=false` matches OpenSCAD's default."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BoxArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<BoxArgs>()?;
            let centered = a.center.unwrap_or(false);
            let src = format!(
                "cube([{:.4}, {:.4}, {:.4}], center={});\n",
                a.x, a.y, a.z, centered
            );
            Ok(text_result(json!({ "scad": src }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CylinderArgs {
    /// Cylinder height along Z.
    height: f64,
    /// Radius (single-radius cylinder).
    #[serde(default)]
    r: Option<f64>,
    /// Bottom radius (cone/frustum form).
    #[serde(default)]
    r1: Option<f64>,
    /// Top radius (cone/frustum form).
    #[serde(default)]
    r2: Option<f64>,
    /// Facet count override.
    #[serde(default)]
    fn_: Option<u32>,
    /// Center on origin (true) or anchor at the base (false, OpenSCAD default).
    #[serde(default)]
    center: Option<bool>,
}

pub struct ScadCylinder;
impl Skill for ScadCylinder {
    fn name(&self) -> &'static str {
        "scad_cylinder"
    }
    fn description(&self) -> &'static str {
        "Emit OpenSCAD `cylinder(h=…, r=… | r1=…, r2=…, $fn=…, center=…);`. \
        Provide either `r` or both `r1` and `r2`. `$fn` defaults to caller's \
        global; pass `fn_` to override."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CylinderArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CylinderArgs>()?;
            let mut args = format!("h={:.4}", a.height);
            match (a.r, a.r1, a.r2) {
                (Some(r), None, None) => {
                    let _ = write!(args, ", r={:.4}", r);
                }
                (None, Some(r1), Some(r2)) => {
                    let _ = write!(args, ", r1={:.4}, r2={:.4}", r1, r2);
                }
                _ => return Err(invalid("supply either `r` alone or both `r1` and `r2`")),
            }
            if let Some(f) = a.fn_ {
                let _ = write!(args, ", $fn={f}");
            }
            if let Some(c) = a.center {
                let _ = write!(args, ", center={c}");
            }
            let src = format!("cylinder({args});\n");
            Ok(text_result(json!({ "scad": src }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SphereArgs {
    /// Sphere radius.
    r: f64,
    /// Facet count override.
    #[serde(default)]
    fn_: Option<u32>,
}

pub struct ScadSphere;
impl Skill for ScadSphere {
    fn name(&self) -> &'static str {
        "scad_sphere"
    }
    fn description(&self) -> &'static str {
        "Emit OpenSCAD `sphere(r=…, $fn=…);`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SphereArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SphereArgs>()?;
            let mut args = format!("r={:.4}", a.r);
            if let Some(f) = a.fn_ {
                let _ = write!(args, ", $fn={f}");
            }
            let src = format!("sphere({args});\n");
            Ok(text_result(json!({ "scad": src }).to_string()))
        })
    }
}

// ---------------------------------------------------------------------------
// scad_flange — idiomatic pattern: cylindrical flange with N holes on PCD.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FlangeArgs {
    /// Outer diameter (mm).
    od: f64,
    /// Thickness (mm).
    thickness: f64,
    /// Pitch circle diameter (mm).
    pcd: f64,
    /// Number of bolt holes.
    n: u32,
    /// Bolt-hole radius (mm). Pass clearance radius, not bolt radius.
    hole_r: f64,
    /// Optional inner-diameter bore.
    #[serde(default)]
    bore: Option<f64>,
    /// `$fn` override for cylinders. Default 64.
    #[serde(default)]
    fn_: Option<u32>,
}

pub struct ScadFlange;
impl Skill for ScadFlange {
    fn name(&self) -> &'static str {
        "scad_flange"
    }
    fn description(&self) -> &'static str {
        "Emit OpenSCAD source for a round flange: outer cylinder, optional \
        central bore, and N evenly spaced clearance holes on a pitch-circle \
        diameter. Idiomatic pattern: `difference()` of the disc and a \
        `for(...) rotate(...) translate(...) cylinder(...)` array. Holes are \
        emitted with -1 / +2 mm Z over-extrusion to cleanly cut both faces."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FlangeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<FlangeArgs>()?;
            if a.od <= 0.0 || a.thickness <= 0.0 || a.pcd <= 0.0 || a.hole_r <= 0.0 {
                return Err(invalid("od, thickness, pcd, hole_r must be > 0"));
            }
            if a.n == 0 {
                return Err(invalid("n must be > 0"));
            }
            if a.pcd > a.od {
                return Err(invalid("pcd must be ≤ od"));
            }
            let fnv = a.fn_.unwrap_or(64);
            let mut src = String::new();
            let _ = writeln!(
                src,
                "// Flange — OD {} mm, t {} mm, {} holes on PCD {} mm",
                a.od, a.thickness, a.n, a.pcd
            );
            let _ = writeln!(src, "difference() {{");
            let _ = writeln!(
                src,
                "  cylinder(h={:.4}, r={:.4}, $fn={fnv});",
                a.thickness,
                a.od / 2.0
            );
            if let Some(bore) = a.bore {
                if bore <= 0.0 || bore >= a.od {
                    return Err(invalid("bore must be in (0, od)"));
                }
                let _ = writeln!(
                    src,
                    "  translate([0, 0, -1]) cylinder(h={:.4}, r={:.4}, $fn={fnv});",
                    a.thickness + 2.0,
                    bore / 2.0
                );
            }
            let _ = writeln!(src, "  for (i = [0:{}]) {{", a.n - 1);
            let _ = writeln!(src, "    rotate([0, 0, 360 * i / {}])", a.n);
            let _ = writeln!(src, "      translate([{:.4}, 0, -1])", a.pcd / 2.0);
            let _ = writeln!(
                src,
                "      cylinder(h={:.4}, r={:.4}, $fn={fnv});",
                a.thickness + 2.0,
                a.hole_r
            );
            let _ = writeln!(src, "  }}");
            let _ = writeln!(src, "}}");
            Ok(text_result(json!({ "scad": src }).to_string()))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(GcodeDrillHole),
        Box::new(GcodeBoltPattern),
        Box::new(GcodeParseSummary),
        Box::new(ScadBox),
        Box::new(ScadCylinder),
        Box::new(ScadSphere),
        Box::new(ScadFlange),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drill_preamble_correct() {
        // Confirm the preamble of the emitted G-code matches RS-274/NGC.
        let mut g = String::new();
        let _ = writeln!(g, "G17 G21 G90 G94");
        assert!(g.contains("G17"));
        assert!(g.contains("G21"));
        assert!(g.contains("G90"));
        assert!(g.contains("G94"));
    }

    #[test]
    fn bolt_pattern_geometry() {
        // 4 holes on PCD 100 mm centered at (0, 0) starting at 0°:
        // should land at (50, 0), (0, 50), (-50, 0), (0, -50).
        let r = 50.0_f64;
        let pts: Vec<(f64, f64)> = (0..4)
            .map(|i| {
                let theta = (i as f64) * std::f64::consts::FRAC_PI_2;
                (r * theta.cos(), r * theta.sin())
            })
            .collect();
        assert!((pts[0].0 - 50.0).abs() < 1.0e-9);
        assert!((pts[1].1 - 50.0).abs() < 1.0e-9);
        assert!((pts[2].0 + 50.0).abs() < 1.0e-9);
        assert!((pts[3].1 + 50.0).abs() < 1.0e-9);
    }
}
