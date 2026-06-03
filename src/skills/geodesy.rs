//! Geodesy and coordinate-system skill — full WGS84 ellipsoidal suite via
//! Karney's `geographiclib-rs`. On by default (pure math, no host
//! requirement).
//!
//! Tools (geodesic primitives):
//!   - `geo_vincenty_inverse`  — point-to-point distance + initial / final bearing on WGS84.
//!   - `geo_vincenty_direct`   — destination + final bearing from start + bearing + distance.
//!   - `geo_great_circle_polyline` — N intermediate points along a geodesic.
//!   - `geo_cross_track`       — perpendicular distance + along-track distance from a path.
//!   - `geo_polygon_area_geodesic` — exact polygon area on the ellipsoid.
//!
//! Tools (coordinate-system conversions):
//!   - `geo_mgrs_from_latlon`  / `geo_latlon_from_mgrs`  — MGRS / USNG.
//!   - `geo_utm_from_latlon`   / `geo_latlon_from_utm`   — UTM zone, hemisphere, easting, northing.
//!   - `geo_ecef_from_latlon`  / `geo_latlon_from_ecef`  — geocentric ECEF.
//!
//! Tools (datum):
//!   - `geo_helmert` — 7-parameter Helmert datum transform (translation, rotation, scale).

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use geographiclib_rs::{Geodesic, InverseGeodesic, PolygonArea, Winding};
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

/// WGS84 semi-major axis (m).
const WGS84_A: f64 = 6_378_137.0;
/// WGS84 first flattening.
const WGS84_F: f64 = 1.0 / 298.257_223_563;

fn wgs84() -> Geodesic {
    Geodesic::wgs84()
}

// ---------------------------------------------------------------------------
// Geodesic primitives
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct InverseArgs {
    /// Start latitude (degrees, −90..90).
    lat1: f64,
    /// Start longitude (degrees, −180..180).
    lon1: f64,
    /// End latitude (degrees, −90..90).
    lat2: f64,
    /// End longitude (degrees, −180..180).
    lon2: f64,
}

pub struct GeoVincentyInverse;
impl Skill for GeoVincentyInverse {
    fn name(&self) -> &'static str {
        "geo_vincenty_inverse"
    }
    fn description(&self) -> &'static str {
        "Solve the geodesic inverse problem on WGS84: given two points return \
        `distance_m` (ellipsoidal distance) and `azi1_deg` / `azi2_deg` \
        (initial and final azimuths, true north, 0..360). Karney's algorithm — \
        accurate to machine precision, including antipodal cases Vincenty fails on."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<InverseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<InverseArgs>()?;
            let g = wgs84();
            let (s12, azi1, azi2): (f64, f64, f64) = g.inverse(a.lat1, a.lon1, a.lat2, a.lon2);
            Ok(text_result(
                json!({
                    "distance_m": s12,
                    "azi1_deg": (azi1 + 360.0) % 360.0,
                    "azi2_deg": (azi2 + 360.0) % 360.0,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "London to Paris",
                args: r#"{"lat1": 51.5074, "lon1": -0.1278, "lat2": 48.8566, "lon2": 2.3522}"#,
                note: Some(
                    "Returns ellipsoidal distance in meters plus initial/final azimuth in degrees.",
                ),
            },
            SkillExample {
                title: "Near-antipodal pair (Vincenty would fail)",
                args: r#"{"lat1": 0.0, "lon1": 0.0, "lat2": 0.5, "lon2": 179.5}"#,
                note: Some("Karney's algorithm converges where classic Vincenty diverges."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Get accurate WGS84 distance + initial bearing between two points.",
            "Compute final azimuth at the destination (different from initial on a great circle).",
            "Handle antipodal / near-antipodal cases reliably.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "lat1",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "lon1",
                min: Some(-180.0),
                max: Some(180.0),
            },
            Rule::Range {
                field: "lat2",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "lon2",
                min: Some(-180.0),
                max: Some(180.0),
            },
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DirectArgs {
    /// Start latitude (degrees).
    lat: f64,
    /// Start longitude (degrees).
    lon: f64,
    /// Initial azimuth from start (degrees, true north).
    azimuth_deg: f64,
    /// Distance to travel along the geodesic (meters; ≥ 0).
    distance_m: f64,
}

pub struct GeoVincentyDirect;
impl Skill for GeoVincentyDirect {
    fn name(&self) -> &'static str {
        "geo_vincenty_direct"
    }
    fn description(&self) -> &'static str {
        "Solve the geodesic direct problem on WGS84: given a start point, \
        initial azimuth, and distance, return the destination (`lat`, `lon`) \
        and final azimuth (`azi_final_deg`)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DirectArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use geographiclib_rs::DirectGeodesic;
            let (_s, a) = ctx.parse::<DirectArgs>()?;
            let g = wgs84();
            let (lat2, lon2, azi2): (f64, f64, f64) =
                g.direct(a.lat, a.lon, a.azimuth_deg, a.distance_m);
            Ok(text_result(
                json!({
                    "lat": lat2,
                    "lon": lon2,
                    "azi_final_deg": (azi2 + 360.0) % 360.0,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "100 km due east from London",
                args: r#"{"lat": 51.5074, "lon": -0.1278, "azimuth_deg": 90, "distance_m": 100000}"#,
                note: Some("Returns the destination (lat, lon) and the final azimuth at arrival."),
            },
            SkillExample {
                title: "Heading north from the equator",
                args: r#"{"lat": 0, "lon": 0, "azimuth_deg": 0, "distance_m": 1112000}"#,
                note: Some("Going north along a meridian; ~10° latitude per 1112 km."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Project a destination point from a start + bearing + distance.",
            "Step along a geodesic when you know the direction, not the endpoint.",
            "Generate flight-plan waypoints by repeatedly applying a direct solution.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "lat",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "lon",
                min: Some(-180.0),
                max: Some(180.0),
            },
            Rule::Range {
                field: "distance_m",
                min: Some(0.0),
                max: None,
            },
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PolylineArgs {
    /// Start latitude (degrees).
    lat1: f64,
    /// Start longitude (degrees).
    lon1: f64,
    /// End latitude (degrees).
    lat2: f64,
    /// End longitude (degrees).
    lon2: f64,
    /// Number of intermediate points to generate (≥ 2). The result includes
    /// the endpoints, so `n=10` returns 10 points.
    n: usize,
}

pub struct GeoGreatCirclePolyline;
impl Skill for GeoGreatCirclePolyline {
    fn name(&self) -> &'static str {
        "geo_great_circle_polyline"
    }
    fn description(&self) -> &'static str {
        "Densify a WGS84 geodesic between two points into `n` evenly spaced \
        samples (including the endpoints). Useful for rendering routes / \
        flight paths / great-circle arcs. Returns `{points: [[lat, lon], ...]}`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PolylineArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            use geographiclib_rs::DirectGeodesic;
            let (_s, a) = ctx.parse::<PolylineArgs>()?;
            let g = wgs84();
            let (s12, azi1, _): (f64, f64, f64) = g.inverse(a.lat1, a.lon1, a.lat2, a.lon2);
            let mut points: Vec<[f64; 2]> = Vec::with_capacity(a.n);
            for i in 0..a.n {
                let frac = i as f64 / (a.n - 1) as f64;
                let (lat, lon, _): (f64, f64, f64) = g.direct(a.lat1, a.lon1, azi1, frac * s12);
                points.push([lat, lon]);
            }
            Ok(text_result(json!({ "points": points }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "10 points along London → Paris",
                args: r#"{"lat1": 51.5074, "lon1": -0.1278, "lat2": 48.8566, "lon2": 2.3522, "n": 10}"#,
                note: Some("Includes both endpoints; `n` must be ≥ 2 and ≤ 10000."),
            },
            SkillExample {
                title: "Coarse 3-point sample",
                args: r#"{"lat1": 0, "lon1": 0, "lat2": 0, "lon2": 90, "n": 3}"#,
                note: Some("First, midpoint, and last point along the geodesic."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Render a great-circle flight path as a polyline on a map.",
            "Densify a geodesic for downstream interpolation or visualization.",
            "Generate intermediate waypoints between two endpoints.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "lat1",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "lon1",
                min: Some(-180.0),
                max: Some(180.0),
            },
            Rule::Range {
                field: "lat2",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "lon2",
                min: Some(-180.0),
                max: Some(180.0),
            },
            Rule::Range {
                field: "n",
                min: Some(2.0),
                max: Some(10_000.0),
            },
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CrossTrackArgs {
    /// Latitude / longitude of the point.
    lat: f64,
    /// Longitude of the point (degrees).
    lon: f64,
    /// Path start latitude (degrees).
    lat1: f64,
    /// Path start longitude (degrees).
    lon1: f64,
    /// Path end latitude (degrees).
    lat2: f64,
    /// Path end longitude (degrees).
    lon2: f64,
}

pub struct GeoCrossTrack;
impl Skill for GeoCrossTrack {
    fn name(&self) -> &'static str {
        "geo_cross_track"
    }
    fn description(&self) -> &'static str {
        "Cross-track (perpendicular) distance and along-track distance from a \
        point to a great-circle path defined by (lat1,lon1) → (lat2,lon2). \
        Uses spherical-earth approximation, accurate to ~0.1% for paths < 10 km \
        and degrades gracefully at continental scale. Returns \
        `cross_track_m` (signed: +right of path, −left), `along_track_m`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CrossTrackArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CrossTrackArgs>()?;
            const R: f64 = 6_371_008.8; // Mean Earth radius (m).
            let to_rad = std::f64::consts::PI / 180.0;
            let lat1 = a.lat1 * to_rad;
            let lon1 = a.lon1 * to_rad;
            let lat = a.lat * to_rad;
            let lon = a.lon * to_rad;
            let lat2 = a.lat2 * to_rad;
            let lon2 = a.lon2 * to_rad;

            let bearing12 = bearing_rad(lat1, lon1, lat2, lon2);
            let bearing13 = bearing_rad(lat1, lon1, lat, lon);
            let d13 = haversine(lat1, lon1, lat, lon) / R;
            let xt = (d13.sin() * (bearing13 - bearing12).sin()).asin();
            let cross_track_m = xt * R;
            let along_track_m = (d13.cos() / xt.cos()).acos() * R;
            Ok(text_result(
                json!({
                    "cross_track_m": cross_track_m,
                    "along_track_m": along_track_m,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "How far off-course",
                args: r#"{"lat": 51.0, "lon": 1.0, "lat1": 51.5074, "lon1": -0.1278, "lat2": 48.8566, "lon2": 2.3522}"#,
                note: Some("`cross_track_m` is signed: +right of the path, −left."),
            },
            SkillExample {
                title: "On-track at the start",
                args: r#"{"lat": 51.5074, "lon": -0.1278, "lat1": 51.5074, "lon1": -0.1278, "lat2": 48.8566, "lon2": 2.3522}"#,
                note: Some("Returns near-zero cross-track and zero along-track."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute deviation of a position from a planned great-circle route.",
            "Get along-track progress for a moving aircraft / vessel.",
            "Sanity-check whether a waypoint lies on a leg of a route.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "lat",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "lon",
                min: Some(-180.0),
                max: Some(180.0),
            },
            Rule::Range {
                field: "lat1",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "lon1",
                min: Some(-180.0),
                max: Some(180.0),
            },
            Rule::Range {
                field: "lat2",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "lon2",
                min: Some(-180.0),
                max: Some(180.0),
            },
        ]
    }
}

fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_008.8;
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let a = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

fn bearing_rad(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let dlon = lon2 - lon1;
    let y = dlon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();
    y.atan2(x)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PolygonAreaArgs {
    /// Polygon vertices as [[lat, lon], ...] in order (no closing duplicate).
    vertices: Vec<[f64; 2]>,
}

pub struct GeoPolygonAreaGeodesic;
impl Skill for GeoPolygonAreaGeodesic {
    fn name(&self) -> &'static str {
        "geo_polygon_area_geodesic"
    }
    fn description(&self) -> &'static str {
        "Exact polygon area on WGS84 (Karney). Returns `area_m2` (signed; \
        positive for counterclockwise winding when looking down on the surface) \
        and `perimeter_m`. Vertices are [[lat, lon], …] in order — DON'T \
        duplicate the closing point."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PolygonAreaArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PolygonAreaArgs>()?;
            let g = wgs84();
            let mut p = PolygonArea::new(&g, Winding::CounterClockwise);
            for v in &a.vertices {
                p.add_point(v[0], v[1]);
            }
            let (perimeter, area, _n) = p.compute(false);
            Ok(text_result(
                json!({ "area_m2": area, "perimeter_m": perimeter }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "1° × 1° square near the equator",
                args: r#"{"vertices": [[0,0],[0,1],[1,1],[1,0]]}"#,
                note: Some("Returns area in m² (~1.23e10) and perimeter; counterclockwise winding is positive."),
            },
            SkillExample {
                title: "Triangle of three city points",
                args: r#"{"vertices": [[51.5074,-0.1278],[48.8566,2.3522],[52.5200,13.4050]]}"#,
                note: Some("Don't repeat the first vertex at the end; the polygon closes automatically."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Measure the area of a country / state / parcel on the WGS84 ellipsoid.",
            "Get exact ellipsoidal area where planar projections would distort results.",
            "Pair area and perimeter for a polygon in a single call.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "vertices",
            min: Some(3),
            max: None,
        }]
    }
}

// ---------------------------------------------------------------------------
// UTM (Universal Transverse Mercator)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LatLonArgs {
    /// Latitude in decimal degrees.
    lat: f64,
    /// Longitude in decimal degrees.
    lon: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UtmArgs {
    /// Zone number 1..60.
    zone: u8,
    /// `"N"` (northern) or `"S"` (southern).
    hemisphere: String,
    /// Easting in meters (from the central meridian + 500 000 false easting).
    easting: f64,
    /// Northing in meters (from the equator; 10 000 000 false northing in the southern hemisphere).
    northing: f64,
}

pub struct GeoUtmFromLatLon;
impl Skill for GeoUtmFromLatLon {
    fn name(&self) -> &'static str {
        "geo_utm_from_latlon"
    }
    fn description(&self) -> &'static str {
        "Convert WGS84 (lat, lon) to UTM. Returns zone, hemisphere, easting (m \
        from the central meridian + 500 000 false easting), and northing (m \
        from the equator; 10 000 000 false northing for the southern \
        hemisphere). Valid for ±80° latitude (poles excluded)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<LatLonArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<LatLonArgs>()?;
            let (zone, hemi, easting, northing) = utm_forward(a.lat, a.lon);
            Ok(text_result(
                json!({
                    "zone": zone,
                    "hemisphere": hemi,
                    "easting": easting,
                    "northing": northing,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Seattle to UTM",
                args: r#"{"lat": 47.6062, "lon": -122.3321}"#,
                note: Some("Returns zone, hemisphere, easting, northing in meters."),
            },
            SkillExample {
                title: "Southern hemisphere point",
                args: r#"{"lat": -33.8688, "lon": 151.2093}"#,
                note: Some("Sydney; expect hemisphere `S` and northing > 6 000 000."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Project lat/lon into UTM for a planar grid calculation.",
            "Get an easting / northing pair for cadastral or survey work.",
            "Identify the UTM zone covering a given point.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "lat",
                min: Some(-80.0),
                max: Some(84.0),
            },
            Rule::Range {
                field: "lon",
                min: Some(-180.0),
                max: Some(180.0),
            },
        ]
    }
}

pub struct GeoLatLonFromUtm;
impl Skill for GeoLatLonFromUtm {
    fn name(&self) -> &'static str {
        "geo_latlon_from_utm"
    }
    fn description(&self) -> &'static str {
        "Convert UTM (zone, hemisphere, easting, northing) back to WGS84 \
        (lat, lon)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<UtmArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<UtmArgs>()?;
            let hemi = a.hemisphere.trim().to_uppercase();
            if hemi != "N" && hemi != "S" {
                return Err(invalid("hemisphere must be 'N' or 'S'"));
            }
            let (lat, lon) = utm_inverse(a.zone, hemi.as_str(), a.easting, a.northing);
            Ok(text_result(json!({ "lat": lat, "lon": lon }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "UTM zone 10 N → lat/lon",
                args: r#"{"zone": 10, "hemisphere": "N", "easting": 550000, "northing": 5275000}"#,
                note: Some("Returns decimal-degree latitude / longitude on WGS84."),
            },
            SkillExample {
                title: "Southern hemisphere round-trip input",
                args: r#"{"zone": 56, "hemisphere": "S", "easting": 334897, "northing": 6251896}"#,
                note: Some("Pair with `geo_utm_from_latlon` for round-trip verification."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert a UTM grid reference back to WGS84 latitude / longitude.",
            "Decode survey-data easting / northing into geographic coordinates.",
            "Round-trip a coordinate through UTM for testing.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Range {
            field: "zone",
            min: Some(1.0),
            max: Some(60.0),
        }]
    }
}

/// Karney/USGS UTM forward conversion (series form). Accurate to ~1 mm
/// over the entire valid latitude range.
fn utm_forward(lat: f64, lon: f64) -> (u8, &'static str, f64, f64) {
    let to_rad = std::f64::consts::PI / 180.0;
    let zone = (((lon + 180.0) / 6.0).floor() as i32).clamp(0, 59) as u8 + 1;
    let lon0 = (zone as f64 - 1.0) * 6.0 - 180.0 + 3.0;
    let phi = lat * to_rad;
    let lam = (lon - lon0) * to_rad;

    let a = WGS84_A;
    let f = WGS84_F;
    let n = f / (2.0 - f);
    let n2 = n * n;
    let n3 = n2 * n;
    let n4 = n3 * n;

    let a_bar = a / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0);
    let alpha = [
        n / 2.0 - 2.0 / 3.0 * n2 + 5.0 / 16.0 * n3,
        13.0 / 48.0 * n2 - 3.0 / 5.0 * n3,
        61.0 / 240.0 * n3,
    ];

    let t = phi.sin().atanh()
        - (2.0 * n.sqrt() / (1.0 + n)) * (((2.0 * n.sqrt()) / (1.0 + n)) * phi.sin()).atanh();
    let t_prime = t.sinh().atan2(lam.cos());
    let eta_prime = (lam.sin() / (t.sinh().powi(2) + lam.cos().powi(2)).sqrt()).atanh();

    let mut xi = t_prime;
    let mut eta = eta_prime;
    for (j, alpha_j) in alpha.iter().enumerate() {
        let j = (j + 1) as f64;
        xi += alpha_j * (2.0 * j * t_prime).sin() * (2.0 * j * eta_prime).cosh();
        eta += alpha_j * (2.0 * j * t_prime).cos() * (2.0 * j * eta_prime).sinh();
    }

    let k0 = 0.9996;
    let easting = k0 * a_bar * eta + 500_000.0;
    let mut northing = k0 * a_bar * xi;
    if lat < 0.0 {
        northing += 10_000_000.0;
    }
    let hemi = if lat >= 0.0 { "N" } else { "S" };
    (zone, hemi, easting, northing)
}

/// Inverse UTM (series form).
fn utm_inverse(zone: u8, hemi: &str, easting: f64, northing: f64) -> (f64, f64) {
    let to_deg = 180.0 / std::f64::consts::PI;
    let lon0 = (zone as f64 - 1.0) * 6.0 - 180.0 + 3.0;
    let n_off = if hemi == "S" { 10_000_000.0 } else { 0.0 };

    let a = WGS84_A;
    let f = WGS84_F;
    let n = f / (2.0 - f);
    let n2 = n * n;
    let n3 = n2 * n;
    let n4 = n3 * n;

    let a_bar = a / (1.0 + n) * (1.0 + n2 / 4.0 + n4 / 64.0);
    let beta = [
        n / 2.0 - 2.0 / 3.0 * n2 + 37.0 / 96.0 * n3,
        n2 / 48.0 + n3 / 15.0,
        17.0 / 480.0 * n3,
    ];
    let delta = [
        2.0 * n - 2.0 / 3.0 * n2 - 2.0 * n3,
        7.0 / 3.0 * n2 - 8.0 / 5.0 * n3,
        56.0 / 15.0 * n3,
    ];

    let k0 = 0.9996;
    let xi = (northing - n_off) / (k0 * a_bar);
    let eta = (easting - 500_000.0) / (k0 * a_bar);

    let mut xi_prime = xi;
    let mut eta_prime = eta;
    for (j, beta_j) in beta.iter().enumerate() {
        let j = (j + 1) as f64;
        xi_prime -= beta_j * (2.0 * j * xi).sin() * (2.0 * j * eta).cosh();
        eta_prime -= beta_j * (2.0 * j * xi).cos() * (2.0 * j * eta).sinh();
    }

    let chi = (xi_prime.sin() / eta_prime.cosh()).asin();
    let mut phi = chi;
    for (j, delta_j) in delta.iter().enumerate() {
        let j = (j + 1) as f64;
        phi += delta_j * (2.0 * j * chi).sin();
    }
    let lam_rad = (eta_prime.sinh() / xi_prime.cos()).atan();
    (phi * to_deg, lon0 + lam_rad * to_deg)
}

// ---------------------------------------------------------------------------
// MGRS (Military Grid Reference System) — built atop UTM, plus a 100 km grid
// letter scheme.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MgrsFromLatLonArgs {
    /// Latitude in decimal degrees.
    lat: f64,
    /// Longitude in decimal degrees.
    lon: f64,
    /// Resolution: digits per axis (1..5). `5` = 1 m; `4` = 10 m; `3` = 100 m;
    /// `2` = 1 km; `1` = 10 km. Default `5`.
    #[serde(default)]
    precision: Option<u8>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MgrsToLatLonArgs {
    /// MGRS string (with or without spaces).
    mgrs: String,
}

pub struct GeoMgrsFromLatLon;
impl Skill for GeoMgrsFromLatLon {
    fn name(&self) -> &'static str {
        "geo_mgrs_from_latlon"
    }
    fn description(&self) -> &'static str {
        "Convert WGS84 (lat, lon) to Military Grid Reference System (MGRS / \
        USNG) at the requested precision. Returns the canonical string like \
        `4QFJ12345678` (1 m resolution)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MgrsFromLatLonArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<MgrsFromLatLonArgs>()?;
            let precision = a.precision.unwrap_or(5).clamp(1, 5);
            let mgrs = mgrs_forward(a.lat, a.lon, precision);
            Ok(text_result(json!({ "mgrs": mgrs }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "1 m precision (default)",
                args: r#"{"lat": 40.7510, "lon": -74.0033}"#,
                note: Some("Returns a 10-digit MGRS like `18TWL...`."),
            },
            SkillExample {
                title: "Coarser 1 km resolution",
                args: r#"{"lat": 40.7510, "lon": -74.0033, "precision": 2}"#,
                note: Some("`precision` = digits per axis: 5 = 1 m, 4 = 10 m, …, 1 = 10 km."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Encode a position as an MGRS / USNG grid reference for a field report.",
            "Generate a coarse MGRS cell for area-level filtering.",
            "Produce a shareable string instead of raw decimal coords.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "lat",
                min: Some(-80.0),
                max: Some(84.0),
            },
            Rule::Range {
                field: "lon",
                min: Some(-180.0),
                max: Some(180.0),
            },
        ]
    }
}

pub struct GeoLatLonFromMgrs;
impl Skill for GeoLatLonFromMgrs {
    fn name(&self) -> &'static str {
        "geo_latlon_from_mgrs"
    }
    fn description(&self) -> &'static str {
        "Convert an MGRS string (with or without spaces, any precision) back \
        to WGS84 (lat, lon). The returned point is the SW corner of the \
        grid cell at the input precision."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MgrsToLatLonArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<MgrsToLatLonArgs>()?;
            let (lat, lon) = mgrs_inverse(&a.mgrs).map_err(invalid)?;
            Ok(text_result(json!({ "lat": lat, "lon": lon }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Full-precision MGRS",
                args: r#"{"mgrs": "18TWL8100068000"}"#,
                note: Some("Returns the SW corner of the cell as (lat, lon)."),
            },
            SkillExample {
                title: "Spaces allowed",
                args: r#"{"mgrs": "18T WL 810 680"}"#,
                note: Some("Whitespace is stripped before parsing; precision auto-detected."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Decode an MGRS / USNG string back to WGS84 latitude / longitude.",
            "Drop a grid reference into a system that wants decimal degrees.",
            "Cross-check an MGRS encoded by another tool.",
        ]
    }
}

const MGRS_LAT_BANDS: &[u8] = b"CDEFGHJKLMNPQRSTUVWX"; // 80°S → 84°N in 8° bands (skips I, O).

fn mgrs_lat_band(lat: f64) -> u8 {
    let idx = ((lat + 80.0) / 8.0).floor().clamp(0.0, 19.0) as usize;
    MGRS_LAT_BANDS[idx]
}

fn mgrs_forward(lat: f64, lon: f64, precision: u8) -> String {
    let (zone, _hemi, easting, northing) = utm_forward(lat, lon);
    let band = mgrs_lat_band(lat) as char;

    // 100 000 m grid square ID — 2 letters.
    // Column scheme cycles every 3 zones (zones 1, 4, 7, … → ABCDEFGH).
    let col_letters_sets: [&[u8]; 3] = [b"ABCDEFGH", b"JKLMNPQR", b"STUVWXYZ"];
    // Row scheme alternates **every** zone (NGA TM 8358.1 §3.2.2.3): odd
    // zones use ABC..V, even zones use FGH..E. Earlier code used
    // `(set/3)%2` which flipped the scheme every 3 zones — wrong, and
    // produced incorrect row letters for ~2/3 of all UTM zones.
    let col_letters_idx = ((zone as i32 - 1) % 3) as usize;
    let col_set = col_letters_sets[col_letters_idx];
    let row_letters_sets: [&[u8]; 2] = [
        b"ABCDEFGHJKLMNPQRSTUV", // odd zones
        b"FGHJKLMNPQRSTUVABCDE", // even zones
    ];
    let row_set = row_letters_sets[((zone as i32 - 1) % 2) as usize];
    let col_idx = ((easting / 100_000.0).floor() as i32 - 1).clamp(0, 7) as usize;
    let row_idx = (((northing % 2_000_000.0) / 100_000.0).floor() as i32).clamp(0, 19) as usize;
    let col_letter = col_set[col_idx] as char;
    let row_letter = row_set[row_idx] as char;

    let scale = 10_f64.powi(5 - precision as i32);
    let e_frac = ((easting % 100_000.0) / scale).floor() as i64;
    let n_frac = ((northing % 100_000.0) / scale).floor() as i64;

    format!(
        "{zone}{band}{col_letter}{row_letter}{:0w$}{:0w$}",
        e_frac,
        n_frac,
        w = precision as usize
    )
}

fn mgrs_inverse(s: &str) -> anyhow::Result<(f64, f64)> {
    let s: String = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_uppercase();
    if s.len() < 5 {
        anyhow::bail!("MGRS string too short");
    }
    // Parse zone + band: first 2 or 3 chars are digits (zone) + 1 letter (band).
    let mut zone_end = 0;
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_digit() {
            zone_end = i + 1;
        } else {
            break;
        }
    }
    if !(1..=2).contains(&zone_end) || s.len() < zone_end + 3 {
        anyhow::bail!("malformed MGRS");
    }
    let zone: u8 = s[..zone_end].parse()?;
    let band = s.as_bytes()[zone_end];
    let col_letter = s.as_bytes()[zone_end + 1];
    let row_letter = s.as_bytes()[zone_end + 2];
    let rest = &s[zone_end + 3..];
    if rest.is_empty() || !rest.len().is_multiple_of(2) {
        anyhow::bail!("digits after grid letters must be even-length");
    }
    let half = rest.len() / 2;
    if half > 5 {
        anyhow::bail!("precision must be 1..5 digits per axis");
    }
    let e_str = &rest[..half];
    let n_str = &rest[half..];
    let scale = 10_f64.powi(5 - half as i32);
    let e_part: f64 = e_str.parse::<i64>()? as f64 * scale;
    let n_part: f64 = n_str.parse::<i64>()? as f64 * scale;

    // Row scheme alternates every zone (see comment in mgrs_forward); column
    // scheme cycles every 3 zones.
    let col_letters_sets: [&[u8]; 3] = [b"ABCDEFGH", b"JKLMNPQR", b"STUVWXYZ"];
    let row_letters_sets: [&[u8]; 2] = [b"ABCDEFGHJKLMNPQRSTUV", b"FGHJKLMNPQRSTUVABCDE"];
    let col_set = col_letters_sets[((zone as i32 - 1) % 3) as usize];
    let row_set = row_letters_sets[((zone as i32 - 1) % 2) as usize];
    let col_idx = col_set
        .iter()
        .position(|&c| c == col_letter)
        .ok_or_else(|| anyhow::anyhow!("invalid MGRS column letter"))? as f64;
    let row_idx = row_set
        .iter()
        .position(|&c| c == row_letter)
        .ok_or_else(|| anyhow::anyhow!("invalid MGRS row letter"))? as f64;

    // Approximate northing via the latitude band's center.
    let band_idx = MGRS_LAT_BANDS
        .iter()
        .position(|&b| b == band)
        .ok_or_else(|| anyhow::anyhow!("invalid MGRS latitude band"))? as f64;
    let band_lat_center = -80.0 + band_idx * 8.0 + 4.0;
    let (_, _, _, band_northing) = utm_forward(band_lat_center, 0.0);
    let northing_base = (band_northing / 2_000_000.0).floor() * 2_000_000.0;
    let easting = (col_idx + 1.0) * 100_000.0 + e_part;
    let northing = northing_base + row_idx * 100_000.0 + n_part;

    let hemi = if band >= b'N' { "N" } else { "S" };
    Ok(utm_inverse(zone, hemi, easting, northing))
}

// ---------------------------------------------------------------------------
// ECEF (Earth-Centered Earth-Fixed)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LatLonAltArgs {
    /// Latitude in decimal degrees.
    lat: f64,
    /// Longitude in decimal degrees.
    lon: f64,
    /// Ellipsoidal height in meters (above WGS84, NOT MSL).
    #[serde(default)]
    alt_m: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EcefArgs {
    /// ECEF X coordinate in meters.
    x: f64,
    /// ECEF Y coordinate in meters.
    y: f64,
    /// ECEF Z coordinate in meters.
    z: f64,
}

pub struct GeoEcefFromLatLon;
impl Skill for GeoEcefFromLatLon {
    fn name(&self) -> &'static str {
        "geo_ecef_from_latlon"
    }
    fn description(&self) -> &'static str {
        "WGS84 (lat, lon, alt) → ECEF (x, y, z) in meters. Ellipsoidal \
        height — NOT mean sea level."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<LatLonAltArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<LatLonAltArgs>()?;
            let alt = a.alt_m.unwrap_or(0.0);
            let (x, y, z) = ecef_from_geodetic(a.lat, a.lon, alt);
            Ok(text_result(json!({ "x": x, "y": y, "z": z }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Surface point",
                args: r#"{"lat": 47.6062, "lon": -122.3321}"#,
                note: Some("Omitting `alt_m` defaults to 0 (on the ellipsoid surface)."),
            },
            SkillExample {
                title: "With ellipsoidal height",
                args: r#"{"lat": 47.6062, "lon": -122.3321, "alt_m": 100}"#,
                note: Some("`alt_m` is ellipsoidal height above WGS84, NOT MSL."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert geographic coords to ECEF for satellite / GNSS computation.",
            "Get a Cartesian state vector seed for an orbital propagator.",
            "Feed a Helmert datum transform with ECEF inputs.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "lat",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "lon",
                min: Some(-180.0),
                max: Some(180.0),
            },
        ]
    }
}

pub struct GeoLatLonFromEcef;
impl Skill for GeoLatLonFromEcef {
    fn name(&self) -> &'static str {
        "geo_latlon_from_ecef"
    }
    fn description(&self) -> &'static str {
        "ECEF (x, y, z) → WGS84 (lat, lon, alt) via Bowring's **closed-form** \
        method (Bowring 1976, *Survey Review* 23:323). Single non-iterative \
        pass; accurate to ≈ 0.1 mm in latitude for all altitudes typical of \
        Earth-orbit work. Near the poles (z → 0 latitude singularity) the \
        formulation degrades — for sub-mm precision at the pole use the \
        Heikkinen 1982 or Vermeille 2002 algorithms."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EcefArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EcefArgs>()?;
            let (lat, lon, alt) = geodetic_from_ecef(a.x, a.y, a.z);
            Ok(text_result(
                json!({ "lat": lat, "lon": lon, "alt_m": alt }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "ECEF round-trip target",
                args: r#"{"x": -2295678.5, "y": -3638263.0, "z": 4691651.0}"#,
                note: Some("Returns lat / lon (degrees) and ellipsoidal altitude in meters."),
            },
            SkillExample {
                title: "Point on the equator",
                args: r#"{"x": 6378137.0, "y": 0.0, "z": 0.0}"#,
                note: Some(
                    "Returns (0°, 0°, ~0 m) — the WGS84 prime-meridian / equator intersection.",
                ),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert an ECEF state vector back to geographic lat / lon / alt.",
            "Decode GNSS / orbital propagator output into mappable coordinates.",
            "Round-trip a coordinate through ECEF as a sanity check.",
        ]
    }
}

fn ecef_from_geodetic(lat_deg: f64, lon_deg: f64, alt_m: f64) -> (f64, f64, f64) {
    let to_rad = std::f64::consts::PI / 180.0;
    let lat = lat_deg * to_rad;
    let lon = lon_deg * to_rad;
    let a = WGS84_A;
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let n_prime = a / (1.0 - e2 * lat.sin().powi(2)).sqrt();
    let x = (n_prime + alt_m) * lat.cos() * lon.cos();
    let y = (n_prime + alt_m) * lat.cos() * lon.sin();
    let z = (n_prime * (1.0 - e2) + alt_m) * lat.sin();
    (x, y, z)
}

fn geodetic_from_ecef(x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    let to_deg = 180.0 / std::f64::consts::PI;
    let a = WGS84_A;
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let b = a * (1.0 - WGS84_F);
    let ep2 = (a * a - b * b) / (b * b);
    let p = (x * x + y * y).sqrt();
    let theta = (z * a).atan2(p * b);
    let lon = y.atan2(x);
    let lat = (z + ep2 * b * theta.sin().powi(3)).atan2(p - e2 * a * theta.cos().powi(3));
    let n_prime = a / (1.0 - e2 * lat.sin().powi(2)).sqrt();
    let alt = p / lat.cos() - n_prime;
    (lat * to_deg, lon * to_deg, alt)
}

// ---------------------------------------------------------------------------
// Helmert 7-parameter datum transform
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HelmertArgs {
    /// Source ECEF X (meters).
    x: f64,
    /// Source ECEF Y (meters).
    y: f64,
    /// Source ECEF Z (meters).
    z: f64,
    /// X translation (meters).
    tx: f64,
    /// Y translation (meters).
    ty: f64,
    /// Z translation (meters).
    tz: f64,
    /// Rotations in arc-seconds. Sign convention: position-vector (Helmert)
    /// — positive rotation is counterclockwise looking down the +axis.
    rx_arcsec: f64,
    /// Y rotation in arc-seconds (position-vector convention).
    ry_arcsec: f64,
    /// Z rotation in arc-seconds (position-vector convention).
    rz_arcsec: f64,
    /// Scale in parts per million (positive scales up).
    scale_ppm: f64,
}

pub struct GeoHelmert;
impl Skill for GeoHelmert {
    fn name(&self) -> &'static str {
        "geo_helmert"
    }
    fn description(&self) -> &'static str {
        "Apply a 7-parameter Helmert datum transform to ECEF coordinates: \
        translation + rotation + scale. Position-vector convention. Used to \
        convert between WGS84 / NAD83 / ITRF / OSGB36 / etc. when you know \
        the 7 parameters for the target frame. Returns transformed ECEF."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HelmertArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<HelmertArgs>()?;
            let arc = std::f64::consts::PI / (180.0 * 3600.0);
            let rx = a.rx_arcsec * arc;
            let ry = a.ry_arcsec * arc;
            let rz = a.rz_arcsec * arc;
            let s = 1.0 + a.scale_ppm * 1e-6;
            let x2 = a.tx + s * (a.x - rz * a.y + ry * a.z);
            let y2 = a.ty + s * (rz * a.x + a.y - rx * a.z);
            let z2 = a.tz + s * (-ry * a.x + rx * a.y + a.z);
            Ok(text_result(
                json!({ "x": x2, "y": y2, "z": z2 }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Identity transform (sanity check)",
                args: r#"{"x": 1000000, "y": 2000000, "z": 3000000, "tx": 0, "ty": 0, "tz": 0, "rx_arcsec": 0, "ry_arcsec": 0, "rz_arcsec": 0, "scale_ppm": 0}"#,
                note: Some("Output equals input when all parameters are zero."),
            },
            SkillExample {
                title: "OSGB36 → WGS84 (approximate)",
                args: r#"{"x": 3874938, "y": -116218, "z": 5047168, "tx": 446.448, "ty": -125.157, "tz": 542.060, "rx_arcsec": 0.150, "ry_arcsec": 0.247, "rz_arcsec": 0.842, "scale_ppm": -20.489}"#,
                note: Some("Classic UK datum-shift parameters; position-vector convention."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert ECEF between datums (WGS84 ↔ NAD83 / OSGB36 / ITRF) given 7 params.",
            "Apply a published Helmert transform without writing the matrix math by hand.",
            "Chain with `geo_ecef_from_latlon` / `geo_latlon_from_ecef` for end-to-end datum shifts.",
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(GeoVincentyInverse),
        Box::new(GeoVincentyDirect),
        Box::new(GeoGreatCirclePolyline),
        Box::new(GeoCrossTrack),
        Box::new(GeoPolygonAreaGeodesic),
        Box::new(GeoUtmFromLatLon),
        Box::new(GeoLatLonFromUtm),
        Box::new(GeoMgrsFromLatLon),
        Box::new(GeoLatLonFromMgrs),
        Box::new(GeoEcefFromLatLon),
        Box::new(GeoLatLonFromEcef),
        Box::new(GeoHelmert),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // TODO: re-tune Vincenty / UTM / Helmert numeric tests against
    // independent ground truth (NGS NCAT, NOAA online calculators).

    #[test]
    fn ecef_roundtrip() {
        let (x, y, z) = ecef_from_geodetic(47.6062, -122.3321, 100.0);
        let (lat, lon, alt) = geodetic_from_ecef(x, y, z);
        assert!((lat - 47.6062).abs() < 1e-7);
        assert!((lon - (-122.3321)).abs() < 1e-7);
        assert!((alt - 100.0).abs() < 1e-3);
    }

    #[test]
    fn mgrs_forward_known() {
        // The Wikipedia example: (40.7510° N, 74.0033° W) → 18T WL 81000 06800-ish.
        let s = mgrs_forward(40.7510, -74.0033, 4);
        assert!(s.starts_with("18T"), "got {s}");
    }

    #[test]
    fn polygon_area_basic() {
        // 1° × 1° lat-lon square near the equator should be roughly 12321 km².
        let g = wgs84();
        let mut p = PolygonArea::new(&g, Winding::CounterClockwise);
        p.add_point(0.0, 0.0);
        p.add_point(0.0, 1.0);
        p.add_point(1.0, 1.0);
        p.add_point(1.0, 0.0);
        let (_per, area, _n) = p.compute(false);
        assert!((area.abs() - 12_308_778_361.0).abs() < 1e8);
    }
}
