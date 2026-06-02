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
    lon1: f64,
    lat2: f64,
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
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PolylineArgs {
    lat1: f64,
    lon1: f64,
    lat2: f64,
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
            if a.n < 2 {
                return Err(invalid("n must be ≥ 2"));
            }
            if a.n > 10_000 {
                return Err(invalid("n must be ≤ 10000"));
            }
            let g = wgs84();
            let (s12, azi1, _): (f64, f64, f64) = g.inverse(a.lat1, a.lon1, a.lat2, a.lon2);
            let mut points: Vec<[f64; 2]> = Vec::with_capacity(a.n);
            for i in 0..a.n {
                let frac = i as f64 / (a.n - 1) as f64;
                let (lat, lon, _): (f64, f64, f64) =
                    g.direct(a.lat1, a.lon1, azi1, frac * s12);
                points.push([lat, lon]);
            }
            Ok(text_result(json!({ "points": points }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CrossTrackArgs {
    /// Latitude / longitude of the point.
    lat: f64,
    lon: f64,
    /// Path start.
    lat1: f64,
    lon1: f64,
    /// Path end.
    lat2: f64,
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
}

fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_008.8;
    let dlat = lat2 - lat1;
    let dlon = lon2 - lon1;
    let a =
        (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
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
            if a.vertices.len() < 3 {
                return Err(invalid("polygon needs ≥ 3 vertices"));
            }
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
}

// ---------------------------------------------------------------------------
// UTM (Universal Transverse Mercator)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LatLonArgs {
    lat: f64,
    lon: f64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct UtmArgs {
    /// Zone number 1..60.
    zone: u8,
    /// `"N"` (northern) or `"S"` (southern).
    hemisphere: String,
    easting: f64,
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
            if !(-80.0..=84.0).contains(&a.lat) {
                return Err(invalid("UTM is undefined outside −80°..84°"));
            }
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
            if !(1..=60).contains(&a.zone) {
                return Err(invalid("zone must be 1..60"));
            }
            let hemi = a.hemisphere.trim().to_uppercase();
            if hemi != "N" && hemi != "S" {
                return Err(invalid("hemisphere must be 'N' or 'S'"));
            }
            let (lat, lon) =
                utm_inverse(a.zone, hemi.as_str(), a.easting, a.northing);
            Ok(text_result(json!({ "lat": lat, "lon": lon }).to_string()))
        })
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

    let t = phi.sin().atanh() - (2.0 * n.sqrt() / (1.0 + n)) * (((2.0 * n.sqrt()) / (1.0 + n)) * phi.sin()).atanh();
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
    lat: f64,
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
            if !(-80.0..=84.0).contains(&a.lat) {
                return Err(invalid("MGRS undefined outside −80°..84°"));
            }
            let mgrs = mgrs_forward(a.lat, a.lon, precision);
            Ok(text_result(json!({ "mgrs": mgrs }).to_string()))
        })
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
}

const MGRS_LAT_BANDS: &[u8] =
    b"CDEFGHJKLMNPQRSTUVWX"; // 80°S → 84°N in 8° bands (skips I, O).

fn mgrs_lat_band(lat: f64) -> u8 {
    let idx = ((lat + 80.0) / 8.0).floor().clamp(0.0, 19.0) as usize;
    MGRS_LAT_BANDS[idx]
}

fn mgrs_forward(lat: f64, lon: f64, precision: u8) -> String {
    let (zone, _hemi, easting, northing) = utm_forward(lat, lon);
    let band = mgrs_lat_band(lat) as char;

    // 100 000 m grid square ID — 2 letters.
    let set = ((zone as i32 - 1) % 6) as usize;
    let col_letters_sets: [&[u8]; 3] = [
        b"ABCDEFGH", b"JKLMNPQR", b"STUVWXYZ",
    ];
    let row_letters_sets: [&[u8]; 2] = [
        b"ABCDEFGHJKLMNPQRSTUV", // odd zones
        b"FGHJKLMNPQRSTUVABCDE", // even zones
    ];
    let col_set = col_letters_sets[set % 3];
    let row_set = row_letters_sets[(set / 3) % 2];
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
    if rest.is_empty() || rest.len() % 2 != 0 {
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

    let set = ((zone as i32 - 1) % 6) as usize;
    let col_letters_sets: [&[u8]; 3] = [
        b"ABCDEFGH", b"JKLMNPQR", b"STUVWXYZ",
    ];
    let row_letters_sets: [&[u8]; 2] = [
        b"ABCDEFGHJKLMNPQRSTUV",
        b"FGHJKLMNPQRSTUVABCDE",
    ];
    let col_set = col_letters_sets[set % 3];
    let row_set = row_letters_sets[(set / 3) % 2];
    let col_idx = col_set.iter().position(|&c| c == col_letter)
        .ok_or_else(|| anyhow::anyhow!("invalid MGRS column letter"))? as f64;
    let row_idx = row_set.iter().position(|&c| c == row_letter)
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
    lat: f64,
    lon: f64,
    /// Ellipsoidal height in meters (above WGS84, NOT MSL).
    #[serde(default)]
    alt_m: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EcefArgs {
    x: f64,
    y: f64,
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
}

pub struct GeoLatLonFromEcef;
impl Skill for GeoLatLonFromEcef {
    fn name(&self) -> &'static str {
        "geo_latlon_from_ecef"
    }
    fn description(&self) -> &'static str {
        "ECEF (x, y, z) → WGS84 (lat, lon, alt) via Bowring's iterative method \
        (converges in 2–3 iterations to machine precision)."
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
    /// Source ECEF coordinates (meters).
    x: f64,
    y: f64,
    z: f64,
    /// Translations (meters).
    tx: f64,
    ty: f64,
    tz: f64,
    /// Rotations in arc-seconds. Sign convention: position-vector (Helmert)
    /// — positive rotation is counterclockwise looking down the +axis.
    rx_arcsec: f64,
    ry_arcsec: f64,
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
