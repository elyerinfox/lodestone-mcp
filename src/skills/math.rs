//! Math skills (local, no network): `math_eval` evaluates an arithmetic/scientific
//! expression; `math_solve` solves a single-variable (in `x`) linear or quadratic
//! equation. Built on the `meval` expression evaluator (functions like sqrt, sin,
//! cos, tan, ln, log, exp, abs, floor, ceil; constants `pi`, `e`; `^` for power),
//! which covers arithmetic, geometry formulas, and evaluating algebraic expressions.

use std::sync::{Arc, LazyLock};

use futures::future::BoxFuture;
use regex::Regex;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

// Insert explicit `*` for the common implicit-multiplication cases meval can't
// parse: a number/`)` before the variable `x` (`2x` → `2*x`) or before a paren
// (`2(…)` → `2*(…)`). Safe around function names (`sin`, `max`, `exp`) and
// scientific notation, since those don't put a digit/`)` immediately before `x`/`(`.
static IMPLICIT_VAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9.)])x").unwrap());
static IMPLICIT_PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9.)])\(").unwrap());

fn normalize(s: &str) -> String {
    let s = IMPLICIT_VAR.replace_all(s, "${1}*x");
    IMPLICIT_PAREN.replace_all(&s, "${1}*(").into_owned()
}

/// Tidy a float for display: damp float noise, then shortest round-trip form.
fn fmt_num(x: f64) -> String {
    if !x.is_finite() {
        return x.to_string();
    }
    let r = (x * 1e10).round() / 1e10;
    let r = if r == 0.0 { 0.0 } else { r }; // normalize -0.0
    format!("{r}")
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MathEvalArgs {
    /// The expression to evaluate, e.g. `2 + 3 * (4 - 1)`, `sqrt(2)`,
    /// `sin(pi/2)`, `3.14159 * 5^2` (area of a circle r=5).
    expression: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MathSolveArgs {
    /// A single-variable (in `x`) linear or quadratic equation, e.g.
    /// `2x + 3 = 7`, `x^2 - 5x + 6 = 0`. Without `=`, the expression is set to 0.
    equation: String,
}

pub struct MathEval;
impl Skill for MathEval {
    fn name(&self) -> &'static str {
        "math_eval"
    }
    fn description(&self) -> &'static str {
        "Evaluate a math expression (local, no network): arithmetic, functions (sqrt, sin, cos, \
        tan, ln, log, exp, abs, floor, ceil, …), constants (pi, e), and `^` for powers. Use for \
        arithmetic and for geometry/algebra formulas you write out, e.g. `pi*5^2`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MathEvalArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<MathEvalArgs>()?;
            let value = meval::eval_str(normalize(&args.expression))
                .map_err(|e| invalid(format!("could not evaluate '{}': {e}", args.expression)))?;
            Ok(text_result(format!(
                "{} = {}",
                args.expression.trim(),
                fmt_num(value)
            )))
        })
    }
}

pub struct MathSolve;
impl Skill for MathSolve {
    fn name(&self) -> &'static str {
        "math_solve"
    }
    fn description(&self) -> &'static str {
        "Solve a single-variable (in `x`) linear or quadratic equation, e.g. `2x + 3 = 7` or \
        `x^2 - 5x + 6 = 0`. Reports the root(s) (or notes complex roots / no solution). Local, no \
        network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MathSolveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<MathSolveArgs>()?;
            let out = solve(&args.equation).map_err(invalid)?;
            Ok(text_result(out))
        })
    }
}

/// Solve a linear/quadratic equation in `x`. Coefficients are recovered by
/// sampling f(x) = LHS − RHS at a few points (works regardless of how the
/// polynomial is written), then verified to be at most degree 2.
fn solve(equation: &str) -> Result<String, String> {
    let eq = equation.trim();
    let (lhs, rhs) = match eq.split_once('=') {
        Some((l, r)) => (l.trim(), r.trim()),
        None => (eq, "0"),
    };
    if lhs.is_empty() {
        return Err("the left-hand side is empty".into());
    }
    let src = normalize(&format!("({lhs})-({rhs})"));
    let expr: meval::Expr = src
        .parse()
        .map_err(|e| format!("could not parse equation: {e}"))?;
    let f = expr
        .bind("x")
        .map_err(|e| format!("the equation must be in a single variable `x`: {e}"))?;

    let f0 = f(0.0);
    let f1 = f(1.0);
    let fm1 = f(-1.0);
    let f2 = f(2.0);
    if [f0, f1, fm1, f2].iter().any(|v| !v.is_finite()) {
        return Err("the equation is undefined at the sampled points".into());
    }
    let c = f0;
    let b = (f1 - fm1) / 2.0;
    let a = (f1 + fm1) / 2.0 - f0;

    // Verify degree ≤ 2: a quadratic must reproduce f(2).
    let predicted = a * 4.0 + b * 2.0 + c;
    if (predicted - f2).abs() > 1e-6 * (1.0 + f2.abs()) {
        return Err("only linear and quadratic equations in `x` are supported".into());
    }

    if a.abs() < 1e-12 {
        // Linear: b*x + c = 0.
        if b.abs() < 1e-12 {
            return if c.abs() < 1e-9 {
                Ok("Any x is a solution (identity).".into())
            } else {
                Ok("No solution (contradiction).".into())
            };
        }
        let x = -c / b;
        return Ok(format!("Linear equation. x = {}", fmt_num(x)));
    }

    // Quadratic: a*x² + b*x + c = 0.
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        let re = -b / (2.0 * a);
        let im = (-disc).sqrt() / (2.0 * a).abs();
        return Ok(format!(
            "Quadratic (a={}, b={}, c={}). Complex roots: x = {} ± {}i",
            fmt_num(a),
            fmt_num(b),
            fmt_num(c),
            fmt_num(re),
            fmt_num(im)
        ));
    }
    let sq = disc.sqrt();
    let x1 = (-b + sq) / (2.0 * a);
    let x2 = (-b - sq) / (2.0 * a);
    if (x1 - x2).abs() < 1e-12 {
        Ok(format!(
            "Quadratic (a={}, b={}, c={}). Double root: x = {}",
            fmt_num(a),
            fmt_num(b),
            fmt_num(c),
            fmt_num(x1)
        ))
    } else {
        Ok(format!(
            "Quadratic (a={}, b={}, c={}). Roots: x = {} or x = {}",
            fmt_num(a),
            fmt_num(b),
            fmt_num(c),
            fmt_num(x1),
            fmt_num(x2)
        ))
    }
}

// ---------------------------------------------------------------------------
// Geo: great-circle distance and azimuth (bearing) between two coordinates.
// ---------------------------------------------------------------------------

/// Mean Earth radius (WGS-84 mean), in kilometres.
const EARTH_RADIUS_KM: f64 = 6371.0088;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GeoArgs {
    /// Latitude of the first point, decimal degrees (−90..90).
    lat1: f64,
    /// Longitude of the first point, decimal degrees (−180..180).
    lon1: f64,
    /// Latitude of the second point, decimal degrees (−90..90).
    lat2: f64,
    /// Longitude of the second point, decimal degrees (−180..180).
    lon2: f64,
}

fn valid_coord(lat: f64, lon: f64) -> bool {
    (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) && lat.is_finite()
}

/// Great-circle distance (haversine) in kilometres.
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dphi = (lat2 - lat1).to_radians();
    let dlam = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlam / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

/// Initial bearing (forward azimuth) from point 1 to point 2, degrees 0..360.
fn azimuth_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dlam = (lon2 - lon1).to_radians();
    let y = dlam.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dlam.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// 16-point compass label for a bearing.
fn compass(deg: f64) -> &'static str {
    const PTS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    PTS[((deg / 22.5).round() as usize) % 16]
}

pub struct GeoDistance;
impl Skill for GeoDistance {
    fn name(&self) -> &'static str {
        "geo_distance"
    }
    fn description(&self) -> &'static str {
        "Great-circle (haversine) distance between two lat/lon coordinates, in km and miles. Local, \
        no network. Coordinates are decimal degrees."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GeoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, a) = ctx.parse::<GeoArgs>()?;
            if !valid_coord(a.lat1, a.lon1) || !valid_coord(a.lat2, a.lon2) {
                return Err(invalid(
                    "coordinates out of range (lat −90..90, lon −180..180)",
                ));
            }
            let km = haversine_km(a.lat1, a.lon1, a.lat2, a.lon2);
            Ok(text_result(format!(
                "({}, {}) → ({}, {})\n  distance: {} km ({} mi)",
                a.lat1,
                a.lon1,
                a.lat2,
                a.lon2,
                fmt_num((km * 1e3).round() / 1e3),
                fmt_num((km * 0.621371 * 1e3).round() / 1e3),
            )))
        })
    }
}

pub struct GeoAzimuth;
impl Skill for GeoAzimuth {
    fn name(&self) -> &'static str {
        "geo_azimuth"
    }
    fn description(&self) -> &'static str {
        "Initial bearing (forward azimuth) from the first lat/lon to the second, in degrees \
        (0=N, 90=E) with a compass label. Also reports the back azimuth. Local, no network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GeoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, a) = ctx.parse::<GeoArgs>()?;
            if !valid_coord(a.lat1, a.lon1) || !valid_coord(a.lat2, a.lon2) {
                return Err(invalid(
                    "coordinates out of range (lat −90..90, lon −180..180)",
                ));
            }
            let fwd = azimuth_deg(a.lat1, a.lon1, a.lat2, a.lon2);
            let back = azimuth_deg(a.lat2, a.lon2, a.lat1, a.lon1);
            Ok(text_result(format!(
                "({}, {}) → ({}, {})\n  azimuth: {}° ({})\n  back azimuth: {}° ({})",
                a.lat1,
                a.lon1,
                a.lat2,
                a.lon2,
                fmt_num((fwd * 100.0).round() / 100.0),
                compass(fwd),
                fmt_num((back * 100.0).round() / 100.0),
                compass(back),
            )))
        })
    }
}

// ---------------------------------------------------------------------------
// Wave: frequency ↔ wavelength ↔ period (v = f·λ).
// ---------------------------------------------------------------------------

/// Speed of light in vacuum, m/s (default wave speed).
const SPEED_OF_LIGHT: f64 = 299_792_458.0;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaveArgs {
    /// Frequency in hertz. Give exactly one of `frequency_hz` or `wavelength_m`.
    #[serde(default)]
    frequency_hz: Option<f64>,
    /// Wavelength in metres. Give exactly one of `frequency_hz` or `wavelength_m`.
    #[serde(default)]
    wavelength_m: Option<f64>,
    /// Wave speed in m/s. Omit for the speed of light (use ~343 for sound in air).
    #[serde(default)]
    speed_m_s: Option<f64>,
}

/// SI-scale a value with a unit (e.g. 1.2e6 Hz → "1.2 MHz").
fn si(value: f64, unit: &str) -> String {
    let abs = value.abs();
    let (scaled, prefix) = if abs >= 1e9 {
        (value / 1e9, "G")
    } else if abs >= 1e6 {
        (value / 1e6, "M")
    } else if abs >= 1e3 {
        (value / 1e3, "k")
    } else if abs >= 1.0 || abs == 0.0 {
        (value, "")
    } else if abs >= 1e-3 {
        (value * 1e3, "m")
    } else if abs >= 1e-6 {
        (value * 1e6, "µ")
    } else {
        (value * 1e9, "n")
    };
    format!("{} {prefix}{unit}", fmt_num((scaled * 1e6).round() / 1e6))
}

pub struct WaveFrequency;
impl Skill for WaveFrequency {
    fn name(&self) -> &'static str {
        "wave_frequency"
    }
    fn description(&self) -> &'static str {
        "Convert between a wave's frequency, wavelength, and period using v = f·λ (local, no \
        network). Give exactly one of frequency_hz or wavelength_m; speed_m_s defaults to the speed \
        of light (set ~343 for sound in air). Returns frequency, wavelength, and period."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WaveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, a) = ctx.parse::<WaveArgs>()?;
            let v = a.speed_m_s.unwrap_or(SPEED_OF_LIGHT);
            if v <= 0.0 {
                return Err(invalid("speed_m_s must be positive"));
            }
            let (freq, wavelength) = match (a.frequency_hz, a.wavelength_m) {
                (Some(f), None) if f > 0.0 => (f, v / f),
                (None, Some(w)) if w > 0.0 => (v / w, w),
                (Some(_), Some(_)) => {
                    return Err(invalid("give only one of frequency_hz / wavelength_m"))
                }
                _ => return Err(invalid("give a positive frequency_hz or wavelength_m")),
            };
            let period = 1.0 / freq;
            Ok(text_result(format!(
                "wave (speed {} m/s):\n  frequency:  {}\n  wavelength: {}\n  period:     {}",
                fmt_num(v),
                si(freq, "Hz"),
                si(wavelength, "m"),
                si(period, "s"),
            )))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(MathEval),
        Box::new(MathSolve),
        Box::new(GeoDistance),
        Box::new(GeoAzimuth),
        Box::new(WaveFrequency),
    ]
}

#[cfg(test)]
mod tests {
    use super::{azimuth_deg, compass, haversine_km, solve};

    #[test]
    fn geo_distance_and_azimuth() {
        // London → Paris: great-circle ≈ 343 km, initial bearing ≈ 148°.
        let km = haversine_km(51.5074, -0.1278, 48.8566, 2.3522);
        assert!((km - 343.0).abs() < 6.0, "distance was {km}");
        let az = azimuth_deg(51.5074, -0.1278, 48.8566, 2.3522);
        assert!((az - 148.0).abs() < 6.0, "azimuth was {az}");
        assert_eq!(compass(0.0), "N");
        assert_eq!(compass(90.0), "E");
        assert_eq!(compass(180.0), "S");
    }

    #[test]
    fn solves_linear() {
        assert!(solve("2x + 3 = 7").unwrap().contains("x = 2"));
    }

    #[test]
    fn solves_quadratic_two_roots() {
        let out = solve("x^2 - 5x + 6 = 0").unwrap();
        assert!(out.contains("x = 3") && out.contains("x = 2"));
    }

    #[test]
    fn rejects_higher_degree() {
        assert!(solve("x^3 = 8").is_err());
    }
}
