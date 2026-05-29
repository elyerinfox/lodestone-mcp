//! Satellite trajectory skills (local compute): propagate a Two-Line Element set
//! with SGP4 to a time and report the ground sub-point (lat/lon/alt), or the
//! azimuth/elevation/range from an observer. `sat_tle` fetches a current TLE from
//! CelesTrak (keyless) so the workflow is `sat_tle` → `sat_position`/`sat_observe`.
//!
//! Frames: SGP4 yields a TEME position; we rotate it to ECEF by GMST, then convert
//! to WGS-84 geodetic. Observer look-angles use the topocentric SEZ frame.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use chrono::NaiveDateTime;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

// WGS-84 ellipsoid.
const WGS84_A_KM: f64 = 6378.137;
const WGS84_F: f64 = 1.0 / 298.257223563;

/// Julian Date (UTC) from a civil datetime.
fn julian_date(dt: &NaiveDateTime) -> f64 {
    use chrono::{Datelike, Timelike};
    let (mut y, mut m) = (dt.year() as f64, dt.month() as f64);
    if dt.month() <= 2 {
        y -= 1.0;
        m += 12.0;
    }
    let a = (y / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    let day_frac =
        (dt.hour() as f64 + dt.minute() as f64 / 60.0 + dt.second() as f64 / 3600.0) / 24.0;
    (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + dt.day() as f64 + b - 1524.5
        + day_frac
}

/// Greenwich Mean Sidereal Time in radians (Vallado, IAU-82).
fn gmst_rad(dt: &NaiveDateTime) -> f64 {
    let jd = julian_date(dt);
    let t = (jd - 2451545.0) / 36525.0;
    let deg = 280.46061837 + 360.98564736629 * (jd - 2451545.0) + 0.000387933 * t * t
        - t * t * t / 38_710_000.0;
    deg.rem_euclid(360.0).to_radians()
}

/// Rotate a TEME position (km) to ECEF by GMST.
fn teme_to_ecef(p: [f64; 3], gmst: f64) -> [f64; 3] {
    let (s, c) = gmst.sin_cos();
    [c * p[0] + s * p[1], -s * p[0] + c * p[1], p[2]]
}

/// ECEF (km) → WGS-84 geodetic (lat°, lon°, alt km).
fn ecef_to_geodetic(e: [f64; 3]) -> (f64, f64, f64) {
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let p = (e[0] * e[0] + e[1] * e[1]).sqrt();
    let lon = e[1].atan2(e[0]);
    let mut lat = e[2].atan2(p * (1.0 - e2));
    let mut alt = 0.0;
    for _ in 0..6 {
        let sin = lat.sin();
        let n = WGS84_A_KM / (1.0 - e2 * sin * sin).sqrt();
        alt = p / lat.cos() - n;
        lat = e[2].atan2(p * (1.0 - e2 * n / (n + alt)));
    }
    (lat.to_degrees(), lon.to_degrees(), alt)
}

/// WGS-84 geodetic (lat°, lon°, alt km) → ECEF (km).
fn geodetic_to_ecef(lat_deg: f64, lon_deg: f64, alt_km: f64) -> [f64; 3] {
    let e2 = WGS84_F * (2.0 - WGS84_F);
    let (lat, lon) = (lat_deg.to_radians(), lon_deg.to_radians());
    let (slat, clat) = lat.sin_cos();
    let (slon, clon) = lon.sin_cos();
    let n = WGS84_A_KM / (1.0 - e2 * slat * slat).sqrt();
    [
        (n + alt_km) * clat * clon,
        (n + alt_km) * clat * slon,
        (n * (1.0 - e2) + alt_km) * slat,
    ]
}

/// Topocentric azimuth°, elevation°, range km of `sat_ecef` seen from an observer.
fn look_angles(obs_lat: f64, obs_lon: f64, obs_alt: f64, sat_ecef: [f64; 3]) -> (f64, f64, f64) {
    let obs = geodetic_to_ecef(obs_lat, obs_lon, obs_alt);
    let (dx, dy, dz) = (
        sat_ecef[0] - obs[0],
        sat_ecef[1] - obs[1],
        sat_ecef[2] - obs[2],
    );
    let (slat, clat) = obs_lat.to_radians().sin_cos();
    let (slon, clon) = obs_lon.to_radians().sin_cos();
    let s = slat * clon * dx + slat * slon * dy - clat * dz;
    let e = -slon * dx + clon * dy;
    let z = clat * clon * dx + clat * slon * dy + slat * dz;
    let range = (s * s + e * e + z * z).sqrt();
    let el = (z / range).asin().to_degrees();
    let az = e.atan2(-s).to_degrees().rem_euclid(360.0);
    (az, el, range)
}

/// Parse a user time (RFC3339 or `YYYY-MM-DD HH:MM:SS`) to naive UTC; `None` = now.
fn parse_at(at: Option<&str>) -> Result<NaiveDateTime, McpError> {
    let Some(s) = at.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(chrono::Utc::now().naive_utc());
    };
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.naive_utc());
    }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(dt);
        }
        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Ok(d.and_hms_opt(0, 0, 0).unwrap());
        }
    }
    Err(invalid(format!("could not parse time '{s}' (use RFC3339)")))
}

/// Propagate a TLE to `when`, returning the ECEF position (km) and speed (km/s).
fn propagate(l1: &str, l2: &str, when: &NaiveDateTime) -> Result<([f64; 3], f64)> {
    let elements = sgp4::Elements::from_tle(None, l1.trim().as_bytes(), l2.trim().as_bytes())
        .map_err(|e| anyhow!("invalid TLE: {e}"))?;
    let constants = sgp4::Constants::from_elements(&elements)
        .map_err(|e| anyhow!("could not build SGP4 constants: {e}"))?;
    let minutes = elements
        .datetime_to_minutes_since_epoch(when)
        .map_err(|e| anyhow!("time/epoch error: {e}"))?;
    let pred = constants
        .propagate(minutes)
        .map_err(|e| anyhow!("SGP4 propagation failed: {e}"))?;
    let ecef = teme_to_ecef(pred.position, gmst_rad(when));
    let speed =
        (pred.velocity[0].powi(2) + pred.velocity[1].powi(2) + pred.velocity[2].powi(2)).sqrt();
    Ok((ecef, speed))
}

// --- argument schemas -------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TleArgs {
    /// A satellite NORAD catalog number (e.g. "25544" for the ISS) or a name to
    /// search (e.g. "ISS").
    query: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PositionArgs {
    /// TLE line 1 (the `1 …` line).
    tle_line1: String,
    /// TLE line 2 (the `2 …` line).
    tle_line2: String,
    /// Time as RFC3339 (e.g. `2026-01-01T00:00:00Z`). Omit for now.
    #[serde(default)]
    at: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ObserveArgs {
    /// TLE line 1.
    tle_line1: String,
    /// TLE line 2.
    tle_line2: String,
    /// Observer latitude, decimal degrees.
    observer_lat: f64,
    /// Observer longitude, decimal degrees.
    observer_lon: f64,
    /// Observer altitude above sea level, km. Default 0.
    #[serde(default)]
    observer_alt_km: Option<f64>,
    /// Time as RFC3339. Omit for now.
    #[serde(default)]
    at: Option<String>,
}

pub struct SatTle;
impl Skill for SatTle {
    fn name(&self) -> &'static str {
        "sat_tle"
    }
    fn description(&self) -> &'static str {
        "Fetch a satellite's current Two-Line Element set (TLE) from CelesTrak (keyless) by NORAD \
        catalog number or name. Returns the name and the two TLE lines to pass to sat_position / \
        sat_observe."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TleArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<TleArgs>()?;
            let q = args.query.trim();
            let key = format!("sat_tle|{q}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let param = if !q.is_empty() && q.chars().all(|c| c.is_ascii_digit()) {
                format!("CATNR={q}")
            } else {
                format!("NAME={}", urlencoding(q))
            };
            let url = format!("https://celestrak.org/NORAD/elements/gp.php?{param}&FORMAT=TLE");
            let body = server
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| internal(e.into()))?
                .error_for_status()
                .map_err(|e| internal(e.into()))?
                .text()
                .await
                .map_err(|e| internal(e.into()))?;
            let lines: Vec<&str> = body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect();
            // CelesTrak TLE format is name / line1 / line2 triples.
            if lines.len() < 3 || !lines[1].starts_with('1') || !lines[2].starts_with('2') {
                return Ok(text_result(format!("No TLE found for '{q}'.")));
            }
            let out = format!("{}\n{}\n{}", lines[0], lines[1], lines[2]);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct SatPosition;
impl Skill for SatPosition {
    fn name(&self) -> &'static str {
        "sat_position"
    }
    fn description(&self) -> &'static str {
        "Propagate a TLE with SGP4 to a time (default now) and report the satellite's ground \
        sub-point: latitude, longitude, altitude (km), and orbital speed (km/s). Local compute."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PositionArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<PositionArgs>()?;
            let when = parse_at(args.at.as_deref())?;
            let (ecef, speed) =
                propagate(&args.tle_line1, &args.tle_line2, &when).map_err(invalid)?;
            let (lat, lon, alt) = ecef_to_geodetic(ecef);
            Ok(text_result(format!(
                "At {when} UTC:\n  sub-point: {lat:.4}°, {lon:.4}°\n  altitude: {alt:.1} km\n  speed: {speed:.3} km/s",
            )))
        })
    }
}

pub struct SatObserve;
impl Skill for SatObserve {
    fn name(&self) -> &'static str {
        "sat_observe"
    }
    fn description(&self) -> &'static str {
        "Propagate a TLE to a time (default now) and report the satellite's look-angles from an \
        observer: azimuth°, elevation° (negative = below horizon), and slant range (km). Local."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ObserveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ObserveArgs>()?;
            let when = parse_at(args.at.as_deref())?;
            let (ecef, _) = propagate(&args.tle_line1, &args.tle_line2, &when).map_err(invalid)?;
            let (az, el, range) = look_angles(
                args.observer_lat,
                args.observer_lon,
                args.observer_alt_km.unwrap_or(0.0),
                ecef,
            );
            let visible = if el >= 0.0 {
                "above horizon"
            } else {
                "below horizon"
            };
            Ok(text_result(format!(
                "At {when} UTC, from ({:.4}, {:.4}):\n  azimuth: {az:.1}°\n  elevation: {el:.1}° ({visible})\n  range: {range:.0} km",
                args.observer_lat, args.observer_lon,
            )))
        })
    }
}

/// Minimal percent-encoding for the CelesTrak NAME query.
fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(SatTle),
        Box::new(SatPosition),
        Box::new(SatObserve),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geodetic_roundtrip() {
        for (lat, lon, alt) in [(51.5, -0.12, 0.05), (-33.9, 151.2, 0.0), (0.0, 0.0, 400.0)] {
            let ecef = geodetic_to_ecef(lat, lon, alt);
            let (la, lo, al) = ecef_to_geodetic(ecef);
            assert!((la - lat).abs() < 1e-6, "lat {la} vs {lat}");
            assert!((lo - lon).abs() < 1e-6, "lon {lo} vs {lon}");
            assert!((al - alt).abs() < 1e-3, "alt {al} vs {alt}");
        }
    }

    #[test]
    fn gmst_known_value() {
        // 2000-01-01 12:00:00 UTC (J2000): GMST ≈ 280.46°.
        let dt = NaiveDateTime::parse_from_str("2000-01-01 12:00:00", "%Y-%m-%d %H:%M:%S").unwrap();
        let deg = gmst_rad(&dt).to_degrees();
        assert!((deg - 280.46).abs() < 0.1, "gmst was {deg}");
    }

    #[test]
    fn iss_tle_propagates_to_a_sane_leo_subpoint() {
        // The canonical (checksum-valid) Vallado ISS verification TLE. At its own
        // epoch the sub-point must be a plausible LEO position: altitude ~400 km and
        // |lat| within the ~51.6° inclination.
        let l1 = "1 25544U 98067A   08264.51782528 -.00002182  00000-0 -11606-4 0  2927";
        let l2 = "2 25544  51.6416 247.4627 0006703 130.5360 325.0288 15.72125391563537";
        let elements = sgp4::Elements::from_tle(None, l1.as_bytes(), l2.as_bytes()).unwrap();
        let constants = sgp4::Constants::from_elements(&elements).unwrap();
        let pred = constants.propagate(sgp4::MinutesSinceEpoch(0.0)).unwrap();
        let ecef = teme_to_ecef(pred.position, gmst_rad(&elements.datetime));
        let (lat, _lon, alt) = ecef_to_geodetic(ecef);
        assert!((300.0..=600.0).contains(&alt), "alt {alt} km not LEO");
        assert!(lat.abs() <= 53.0, "lat {lat} exceeds inclination");
    }
}
