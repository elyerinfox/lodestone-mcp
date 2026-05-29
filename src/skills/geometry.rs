//! Geometry skill (local, no network): great-circle `geo_distance` / `geo_azimuth`
//! between coordinates, plus `geometry_formula` / `geometry_formula_list` for named
//! shape/length/area/volume formulas.

use std::sync::{Arc, LazyLock};

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::arithmetic::fmt_num;
use crate::skills::formula::{self, v, Args, Formula};
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

use std::f64::consts::PI;

#[rustfmt::skip]
static FORMULAS: LazyLock<Vec<Formula>> = LazyLock::new(|| {
    vec![
        Formula { id: "pythagorean", category: "geometry", summary: "Pythagoras: c = √(a²+b²)", inputs: vec![v("a",""), v("b","")], out: v("c",""), eval: |a| (a["a"].powi(2)+a["b"].powi(2)).sqrt() },
        Formula { id: "distance_2d", category: "geometry", summary: "2D distance: d = √(Δx²+Δy²)", inputs: vec![v("dx",""), v("dy","")], out: v("d",""), eval: |a| (a["dx"].powi(2)+a["dy"].powi(2)).sqrt() },
        Formula { id: "distance_3d", category: "geometry", summary: "3D distance: d = √(Δx²+Δy²+Δz²)", inputs: vec![v("dx",""), v("dy",""), v("dz","")], out: v("d",""), eval: |a| (a["dx"].powi(2)+a["dy"].powi(2)+a["dz"].powi(2)).sqrt() },
        Formula { id: "circle_area", category: "geometry", summary: "Circle area: A = π·r²", inputs: vec![v("r","")], out: v("A",""), eval: |a| PI*a["r"].powi(2) },
        Formula { id: "circle_circumference", category: "geometry", summary: "Circle circumference: C = 2π·r", inputs: vec![v("r","")], out: v("C",""), eval: |a| 2.0*PI*a["r"] },
        Formula { id: "sphere_volume", category: "geometry", summary: "Sphere volume: V = (4/3)·π·r³", inputs: vec![v("r","")], out: v("V",""), eval: |a| 4.0/3.0*PI*a["r"].powi(3) },
        Formula { id: "sphere_surface_area", category: "geometry", summary: "Sphere surface: A = 4π·r²", inputs: vec![v("r","")], out: v("A",""), eval: |a| 4.0*PI*a["r"].powi(2) },
        Formula { id: "cylinder_volume", category: "geometry", summary: "Cylinder volume: V = π·r²·h", inputs: vec![v("r",""), v("h","")], out: v("V",""), eval: |a| PI*a["r"].powi(2)*a["h"] },
        Formula { id: "cone_volume", category: "geometry", summary: "Cone volume: V = (1/3)·π·r²·h", inputs: vec![v("r",""), v("h","")], out: v("V",""), eval: |a| PI*a["r"].powi(2)*a["h"]/3.0 },
        Formula { id: "triangle_area", category: "geometry", summary: "Triangle area: A = ½·b·h", inputs: vec![v("b",""), v("h","")], out: v("A",""), eval: |a| 0.5*a["b"]*a["h"] },
        Formula { id: "heron_area", category: "geometry", summary: "Heron's formula: A = √(s(s-a)(s-b)(s-c)), s=(a+b+c)/2", inputs: vec![v("a",""), v("b",""), v("c","")], out: v("A",""), eval: |a| { let s=(a["a"]+a["b"]+a["c"])/2.0; (s*(s-a["a"])*(s-a["b"])*(s-a["c"])).sqrt() } },
        Formula { id: "law_of_cosines_side", category: "geometry", summary: "Law of cosines: c = √(a²+b²-2ab·cos(angle_c°))", inputs: vec![v("a",""), v("b",""), v("angle_c","deg")], out: v("c",""), eval: |a| (a["a"].powi(2)+a["b"].powi(2)-2.0*a["a"]*a["b"]*a["angle_c"].to_radians().cos()).sqrt() },
    ]
});

// ---- Geospatial: great-circle distance and azimuth between coordinates ----

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

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dphi = (lat2 - lat1).to_radians();
    let dlam = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dlam / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_KM * a.sqrt().asin()
}

fn azimuth_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dlam = (lon2 - lon1).to_radians();
    let y = dlam.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dlam.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FormulaArgs {
    /// Formula id (see `geometry_formula_list`), e.g. `circle_area`.
    name: String,
    /// Variable values, e.g. `{"r": 5}`.
    #[serde(default)]
    args: Args,
}

pub struct GeometryFormula;
impl Skill for GeometryFormula {
    fn name(&self) -> &'static str {
        "geometry_formula"
    }
    fn description(&self) -> &'static str {
        "Compute a named geometry formula (areas, volumes, distances, Pythagoras, Heron, law of \
        cosines). Pass `name` (see geometry_formula_list) and `args` as a {var: value} map; angles \
        in degrees."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FormulaArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<FormulaArgs>()?;
            let out = formula::compute(&FORMULAS, &args.name, &args.args).map_err(invalid)?;
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListArgs {
    /// Optional id/equation substring filter.
    #[serde(default)]
    filter: Option<String>,
}

pub struct GeometryFormulaList;
impl Skill for GeometryFormulaList {
    fn name(&self) -> &'static str {
        "geometry_formula_list"
    }
    fn description(&self) -> &'static str {
        "List the named geometry formulas (id, equation, signature). Feed an id to geometry_formula."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ListArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ListArgs>()?;
            Ok(text_result(formula::list(
                &FORMULAS,
                args.filter.as_deref(),
            )))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(GeoDistance),
        Box::new(GeoAzimuth),
        Box::new(GeometryFormula),
        Box::new(GeometryFormulaList),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(id: &str, args: &[(&str, f64)]) -> f64 {
        let f = FORMULAS.iter().find(|f| f.id == id).unwrap();
        let map: Args = args.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        (f.eval)(&map)
    }

    #[test]
    fn geo_distance_and_azimuth() {
        // London → Paris: great-circle ≈ 343 km, initial bearing ≈ 148°.
        let km = haversine_km(51.5074, -0.1278, 48.8566, 2.3522);
        assert!((km - 343.0).abs() < 6.0, "distance was {km}");
        let az = azimuth_deg(51.5074, -0.1278, 48.8566, 2.3522);
        assert!((az - 148.0).abs() < 6.0, "azimuth was {az}");
        assert_eq!(compass(0.0), "N");
        assert_eq!(compass(90.0), "E");
    }

    #[test]
    fn geometry_formulas() {
        assert_eq!(run("pythagorean", &[("a", 3.0), ("b", 4.0)]), 5.0);
        assert!((run("circle_area", &[("r", 1.0)]) - PI).abs() < 1e-9);
        assert!((run("heron_area", &[("a", 3.0), ("b", 4.0), ("c", 5.0)]) - 6.0).abs() < 1e-9);
    }
}
