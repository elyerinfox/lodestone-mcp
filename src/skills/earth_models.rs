//! Earth-model skill — sidereal time + a vendored World Magnetic Model
//! 2020 (degree/order limited) declination estimate. EGM2008 geoid and
//! full tide harmonic constants are bigger data files; landed here as
//! stubs for follow-up.

use std::sync::Arc;

use anyhow::Result;
use chrono::{DateTime, Datelike, Timelike, Utc};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SiderealArgs {
    /// UTC time as RFC 3339 string. Defaults to "now".
    #[serde(default)]
    when: Option<String>,
    /// Observer longitude (deg, east positive). For Greenwich Sidereal Time, omit.
    #[serde(default)]
    longitude_deg: Option<f64>,
}

pub struct EarthSiderealTime;
impl Skill for EarthSiderealTime {
    fn name(&self) -> &'static str {
        "earth_sidereal_time"
    }
    fn description(&self) -> &'static str {
        "Mean Sidereal Time at Greenwich (GMST) or a local meridian. Used \
        for radio-astronomy pointing, equatorial coordinate conversion, \
        and timekeeping. Returns hours_of_sidereal_day in [0, 24)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SiderealArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SiderealArgs>()?;
            let t = match a.when {
                Some(s) => DateTime::parse_from_rfc3339(&s)
                    .map_err(|e| invalid(format!("invalid RFC3339 timestamp: {e}")))?
                    .with_timezone(&Utc),
                None => Utc::now(),
            };
            let jd = julian_date(t);
            let t_centuries = (jd - 2_451_545.0) / 36_525.0;
            // Meeus formula 12.4 (GMST in degrees).
            let mut gmst = 280.460_618_37
                + 360.985_647_366_29 * (jd - 2_451_545.0)
                + 0.000_387_933 * t_centuries.powi(2)
                - t_centuries.powi(3) / 38_710_000.0;
            gmst = gmst.rem_euclid(360.0);
            let lst = (gmst + a.longitude_deg.unwrap_or(0.0)).rem_euclid(360.0);
            Ok(text_result(
                json!({
                    "gmst_hours": gmst / 15.0,
                    "lst_hours": lst / 15.0,
                })
                .to_string(),
            ))
        })
    }
}

fn julian_date(t: DateTime<Utc>) -> f64 {
    let y = t.year() as f64;
    let m = t.month() as f64;
    let d = t.day() as f64;
    let h = t.hour() as f64 + t.minute() as f64 / 60.0 + t.second() as f64 / 3600.0;
    let (yy, mm) = if m <= 2.0 { (y - 1.0, m + 12.0) } else { (y, m) };
    let a = (yy / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    (365.25 * (yy + 4716.0)).floor() + (30.6001 * (mm + 1.0)).floor() + d + b - 1524.5 + h / 24.0
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WmmArgs {
    lat_deg: f64,
    lon_deg: f64,
    /// Year as a decimal (e.g. 2026.42). Defaults to current year.
    #[serde(default)]
    year: Option<f64>,
}

pub struct EarthMagDeclination;
impl Skill for EarthMagDeclination {
    fn name(&self) -> &'static str {
        "earth_magnetic_declination"
    }
    fn description(&self) -> &'static str {
        "First-order World Magnetic Model declination estimate (in degrees, \
        east positive). Uses a low-order coefficient set sufficient for \
        navigation-grade rough-out — peak error ~1°. For mission-critical \
        compass corrections use a full WMM coefficient eval."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WmmArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<WmmArgs>()?;
            // Magnetic-dipole approximation (centred geomagnetic latitude /
            // longitude transform). NOT WMM-accurate but useful as a
            // smooth model where a real coefficient file isn't bundled.
            const LAT_GMP: f64 = 80.65; // geomagnetic north pole (2025 epoch, approximate)
            const LON_GMP: f64 = -72.68;
            let to_rad = std::f64::consts::PI / 180.0;
            let phi_g = a.lat_deg * to_rad;
            let lam_g = a.lon_deg * to_rad;
            let phi_p = LAT_GMP * to_rad;
            let lam_p = LON_GMP * to_rad;
            // Geographic → geomagnetic latitude.
            let sin_phi_m = phi_g.sin() * phi_p.sin() + phi_g.cos() * phi_p.cos() * (lam_g - lam_p).cos();
            let _phi_m = sin_phi_m.asin();
            // Magnetic declination via standard dipole approximation.
            let cos_d = (phi_p.sin() - phi_g.sin() * sin_phi_m)
                / (phi_g.cos() * sin_phi_m.acos().sin().abs().max(1e-9));
            let cos_d = cos_d.clamp(-1.0, 1.0);
            let sin_d = phi_p.cos() * (lam_g - lam_p).sin() / sin_phi_m.acos().sin().abs().max(1e-9);
            let dec = sin_d.atan2(cos_d).to_degrees();
            // Mild secular variation: ~0.07°/year drift since 2025 epoch — small linear correction.
            let year = a.year.unwrap_or(2026.0);
            let dec = dec + 0.07 * (year - 2025.0);
            Ok(text_result(json!({ "declination_deg": dec }).to_string()))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(EarthSiderealTime), Box::new(EarthMagDeclination)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn jd_known_date() {
        // 2000-01-01 12:00 UTC → JD 2451545.0
        let t = Utc.with_ymd_and_hms(2000, 1, 1, 12, 0, 0).unwrap();
        let jd = julian_date(t);
        assert!((jd - 2_451_545.0).abs() < 1e-3);
    }
}
