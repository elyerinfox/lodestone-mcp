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

// ---------------------------------------------------------------------------
// sat_passes — predict upcoming visible passes over an observer
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PassesArgs {
    /// TLE line 1.
    tle_line1: String,
    /// TLE line 2.
    tle_line2: String,
    /// Observer latitude (decimal degrees).
    observer_lat: f64,
    /// Observer longitude (decimal degrees).
    observer_lon: f64,
    /// Observer altitude in km (default 0).
    #[serde(default)]
    observer_alt_km: Option<f64>,
    /// Start of the search window as RFC3339; omit for now.
    #[serde(default)]
    from: Option<String>,
    /// Search window length in hours (default 24, capped at 168 = one week).
    #[serde(default)]
    hours: Option<f64>,
    /// Minimum peak elevation in degrees to report (default 10°; lower for
    /// HEO/Molniya and other low passes). 0 returns every horizon crossing.
    #[serde(default)]
    min_elevation_deg: Option<f64>,
    /// Max passes to return (default 10, capped at 50).
    #[serde(default)]
    max_passes: Option<u32>,
}

/// 16-point compass label from an azimuth in degrees.
fn compass(az: f64) -> &'static str {
    const POINTS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let i = (((az.rem_euclid(360.0) / 22.5) + 0.5).floor() as usize) % 16;
    POINTS[i]
}

fn fmt_hhmmss(secs: i64) -> String {
    let secs = secs.max(0);
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m}m {s}s")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

/// Compute elevation at one instant; returns None on propagation error.
fn elevation_at(
    l1: &str,
    l2: &str,
    when: &NaiveDateTime,
    obs_lat: f64,
    obs_lon: f64,
    obs_alt: f64,
) -> Option<(f64, f64)> {
    let (ecef, _) = propagate(l1, l2, when).ok()?;
    let (az, el, _) = look_angles(obs_lat, obs_lon, obs_alt, ecef);
    Some((az, el))
}

/// Binary-search the horizon-crossing time between `lo` (below) and `hi` (above)
/// — or vice versa — to one-second precision.
fn refine_horizon(
    l1: &str,
    l2: &str,
    mut lo: NaiveDateTime,
    mut hi: NaiveDateTime,
    obs_lat: f64,
    obs_lon: f64,
    obs_alt: f64,
) -> NaiveDateTime {
    use chrono::Duration;
    for _ in 0..20 {
        let mid = lo + (hi - lo) / 2;
        if (hi - lo) <= Duration::seconds(1) {
            return mid;
        }
        match elevation_at(l1, l2, &mid, obs_lat, obs_lon, obs_alt) {
            Some((_, el)) if el >= 0.0 => hi = mid,
            _ => lo = mid,
        }
    }
    lo
}

#[derive(Clone)]
struct Pass {
    rise: NaiveDateTime,
    rise_az: f64,
    peak: NaiveDateTime,
    peak_az: f64,
    peak_el: f64,
    set: NaiveDateTime,
    set_az: f64,
}

pub struct SatPasses;
impl Skill for SatPasses {
    fn name(&self) -> &'static str {
        "sat_passes"
    }
    fn description(&self) -> &'static str {
        "Predict upcoming VISIBLE PASSES of a satellite over an observer location: rise time and \
        azimuth, peak time and elevation, set time and azimuth, plus pass duration. Scans the \
        TLE forward in `hours` (default 24, max 168) and reports passes above `min_elevation_deg` \
        (default 10°). Use this instead of repeatedly calling sat_observe at guessed times."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PassesArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<PassesArgs>()?;
            let start = parse_at(args.from.as_deref())?;
            let hours = args.hours.unwrap_or(24.0).clamp(0.1, 168.0);
            let min_el = args.min_elevation_deg.unwrap_or(10.0).clamp(0.0, 89.0);
            let cap = args.max_passes.unwrap_or(10).clamp(1, 50) as usize;
            let obs_alt = args.observer_alt_km.unwrap_or(0.0);
            let l1 = &args.tle_line1;
            let l2 = &args.tle_line2;

            // 30 s steps catch passes as short as ~1 min (LEO passes are 3–12 min).
            let step = chrono::Duration::seconds(30);
            let total = chrono::Duration::milliseconds((hours * 3600.0 * 1000.0) as i64);
            let end = start + total;

            let mut passes: Vec<Pass> = Vec::new();
            let mut t = start;
            let mut prev_el =
                match elevation_at(l1, l2, &t, args.observer_lat, args.observer_lon, obs_alt) {
                    Some((_, el)) => el,
                    None => return Err(invalid("invalid TLE — could not propagate")),
                };
            let mut in_pass = prev_el >= 0.0;
            let mut rise_t = t;
            let mut rise_below = t;
            let mut peak_t = t;
            let mut peak_az = 0.0;
            let mut peak_el = prev_el;

            while t < end && passes.len() < cap {
                t += step;
                let Some((az, el)) =
                    elevation_at(l1, l2, &t, args.observer_lat, args.observer_lon, obs_alt)
                else {
                    break;
                };
                if !in_pass && el >= 0.0 {
                    // Rising — refine rise time between rise_below and t.
                    in_pass = true;
                    rise_t = refine_horizon(
                        l1,
                        l2,
                        rise_below,
                        t,
                        args.observer_lat,
                        args.observer_lon,
                        obs_alt,
                    );
                    peak_t = t;
                    peak_az = az;
                    peak_el = el;
                } else if in_pass && el >= 0.0 {
                    if el > peak_el {
                        peak_el = el;
                        peak_az = az;
                        peak_t = t;
                    }
                } else if in_pass && el < 0.0 {
                    // Setting — refine set time between (t - step) and t.
                    let set_t = refine_horizon(
                        l1,
                        l2,
                        t,
                        t - step,
                        args.observer_lat,
                        args.observer_lon,
                        obs_alt,
                    );
                    let rise_az = elevation_at(
                        l1,
                        l2,
                        &rise_t,
                        args.observer_lat,
                        args.observer_lon,
                        obs_alt,
                    )
                    .map(|(a, _)| a)
                    .unwrap_or(0.0);
                    let set_az = elevation_at(
                        l1,
                        l2,
                        &set_t,
                        args.observer_lat,
                        args.observer_lon,
                        obs_alt,
                    )
                    .map(|(a, _)| a)
                    .unwrap_or(0.0);
                    if peak_el >= min_el {
                        passes.push(Pass {
                            rise: rise_t,
                            rise_az,
                            peak: peak_t,
                            peak_az,
                            peak_el,
                            set: set_t,
                            set_az,
                        });
                    }
                    in_pass = false;
                }
                rise_below = if !in_pass { t } else { rise_below };
                prev_el = el;
            }
            let _ = prev_el;

            if passes.is_empty() {
                return Ok(text_result(format!(
                    "No passes ≥ {min_el:.0}° elevation in the next {hours:.1} h from ({:.4}, {:.4}).",
                    args.observer_lat, args.observer_lon
                )));
            }
            // Sort by peak elevation desc to surface the BEST pass at the top, but
            // also report chronologically so the model sees the actual order.
            let mut chrono = passes.clone();
            chrono.sort_by_key(|p| p.rise);
            let best = passes
                .iter()
                .max_by(|a, b| a.peak_el.partial_cmp(&b.peak_el).unwrap())
                .unwrap()
                .clone();
            let mut out = format!(
                "{} pass(es) ≥ {:.0}° over ({:.4}, {:.4}) in {:.1} h from {}.\n\n",
                chrono.len(),
                min_el,
                args.observer_lat,
                args.observer_lon,
                hours,
                start.format("%Y-%m-%d %H:%M UTC")
            );
            out.push_str(&format!(
                "BEST (peak {:.1}°): {} → {} (in {})\n\n",
                best.peak_el,
                best.rise.format("%Y-%m-%d %H:%M:%S UTC"),
                best.set.format("%H:%M:%S"),
                fmt_hhmmss((best.rise - start).num_seconds())
            ));
            for (i, p) in chrono.iter().enumerate() {
                let dur = (p.set - p.rise).num_seconds();
                out.push_str(&format!(
                    "#{}  {}  (in {})\n    rise  {}  az {:>5.1}° ({})\n    peak  {}  el {:>5.1}°  az {:>5.1}° ({})\n    set   {}  az {:>5.1}° ({})\n    duration  {}\n\n",
                    i + 1,
                    p.rise.format("%Y-%m-%d %H:%M:%S UTC"),
                    fmt_hhmmss((p.rise - start).num_seconds()),
                    p.rise.format("%H:%M:%S"),
                    p.rise_az,
                    compass(p.rise_az),
                    p.peak.format("%H:%M:%S"),
                    p.peak_el,
                    p.peak_az,
                    compass(p.peak_az),
                    p.set.format("%H:%M:%S"),
                    p.set_az,
                    compass(p.set_az),
                    fmt_hhmmss(dur),
                ));
            }
            Ok(text_result(out))
        })
    }
}

// ---------------------------------------------------------------------------
// sat_group — fetch a whole CelesTrak group (Starlink, GPS, Iridium, …)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GroupArgs {
    /// CelesTrak group id (e.g. "starlink", "gps-ops", "oneweb", "iridium",
    /// "iridium-NEXT", "weather", "noaa", "amateur", "galileo", "glo-ops",
    /// "stations", "active", "visual", "science", "geo"). Case-insensitive.
    group: String,
    /// Optional case-insensitive name substring to filter the returned TLEs
    /// (e.g. "STARLINK-30" to grab one shell of Starlinks).
    #[serde(default)]
    name_filter: Option<String>,
    /// Max satellites to return (default 25, capped at 500). Use this with
    /// `name_filter` so the prompt doesn't drown — Starlink alone is 5000+.
    #[serde(default)]
    max: Option<u32>,
}

pub struct SatGroup;
impl Skill for SatGroup {
    fn name(&self) -> &'static str {
        "sat_group"
    }
    fn description(&self) -> &'static str {
        "Fetch all TLEs for a CelesTrak constellation group (Starlink, GPS, Iridium, OneWeb, \
        Galileo, GLONASS, weather, NOAA, amateur, science, geo, …). Returns a list of \
        {name, tle_line1, tle_line2} entries you can pass to sat_position / sat_observe / \
        sat_passes — usually together with `name_filter` to scope the haul (Starlink alone is \
        thousands)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GroupArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<GroupArgs>()?;
            let group = args.group.trim().to_ascii_lowercase();
            if group.is_empty() {
                return Err(invalid("group must not be empty"));
            }
            let max = args.max.unwrap_or(25).clamp(1, 500) as usize;
            let cache_key = format!("sat_group|{group}");
            let body = if let Some(c) = server.retrieval_get(&cache_key).await {
                c
            } else {
                let url = format!(
                    "https://celestrak.org/NORAD/elements/gp.php?GROUP={}&FORMAT=tle",
                    urlencoding(&group)
                );
                let resp = server
                    .http
                    .get(&url)
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                    .map_err(|e| internal(anyhow!("CelesTrak fetch: {e}")))?;
                let text = resp
                    .text()
                    .await
                    .map_err(|e| internal(anyhow!("CelesTrak read: {e}")))?;
                if text.trim().is_empty() || text.contains("No GP data found") {
                    return Err(invalid(format!(
                        "no TLEs for group \"{group}\" — check the group id at celestrak.org"
                    )));
                }
                server.retrieval_put(cache_key, &text);
                text
            };

            // TLE-format response: 3 lines per satellite (name, "1 ...", "2 ...").
            let lines: Vec<&str> = body.lines().collect();
            let want = args
                .name_filter
                .as_ref()
                .map(|s| s.trim().to_ascii_uppercase())
                .filter(|s| !s.is_empty());
            let mut sats: Vec<(String, String, String)> = Vec::new();
            let mut i = 0;
            let mut total = 0usize;
            while i + 2 < lines.len() {
                let name = lines[i].trim().to_string();
                let l1 = lines[i + 1].trim().to_string();
                let l2 = lines[i + 2].trim().to_string();
                if l1.starts_with("1 ") && l2.starts_with("2 ") {
                    total += 1;
                    let match_ok = want
                        .as_ref()
                        .is_none_or(|w| name.to_ascii_uppercase().contains(w));
                    if match_ok && sats.len() < max {
                        sats.push((name, l1, l2));
                    }
                    i += 3;
                } else {
                    i += 1;
                }
            }
            if sats.is_empty() {
                return Ok(text_result(format!(
                    "{} satellites in CelesTrak group \"{group}\"; none matched filter {:?}.",
                    total, want
                )));
            }
            let mut out = format!(
                "{} satellites in CelesTrak group \"{group}\"; showing {}",
                total,
                sats.len()
            );
            if let Some(w) = &want {
                out.push_str(&format!(" matching \"{w}\""));
            }
            out.push_str(":\n\n");
            for (n, a, b) in &sats {
                out.push_str(&format!("{n}\n{a}\n{b}\n\n"));
            }
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(SatTle),
        Box::new(SatPosition),
        Box::new(SatObserve),
        Box::new(SatPasses),
        Box::new(SatGroup),
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
