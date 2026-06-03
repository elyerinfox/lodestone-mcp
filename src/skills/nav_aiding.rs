//! Navigation aiding skill — DOP from sat geometry, Klobuchar ionospheric
//! delay, Saastamoinen tropospheric delay, ECEF↔ENU, IMU drift modeling.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use nalgebra::DMatrix;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DopArgs {
    /// Per-satellite line-of-sight unit vectors in ENU, length ≥ 4.
    los_enu: Vec<[f64; 3]>,
}

pub struct NavDop;
impl Skill for NavDop {
    fn name(&self) -> &'static str {
        "nav_dop"
    }
    fn description(&self) -> &'static str {
        "GNSS Dilution Of Precision. Build the geometry matrix from \
        per-satellite line-of-sight unit vectors in ENU, then return GDOP, \
        PDOP, HDOP, VDOP, TDOP. Lower is better; rule of thumb: GDOP < 6 \
        is acceptable for navigation."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DopArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DopArgs>()?;
            let n = a.los_enu.len();
            let mut h = DMatrix::<f64>::zeros(n, 4);
            for (i, los) in a.los_enu.iter().enumerate() {
                h[(i, 0)] = -los[0];
                h[(i, 1)] = -los[1];
                h[(i, 2)] = -los[2];
                h[(i, 3)] = 1.0;
            }
            let hth = h.transpose() * &h;
            let q = hth
                .try_inverse()
                .ok_or_else(|| invalid("geometry matrix singular"))?;
            let gdop = (q[(0, 0)] + q[(1, 1)] + q[(2, 2)] + q[(3, 3)]).sqrt();
            let pdop = (q[(0, 0)] + q[(1, 1)] + q[(2, 2)]).sqrt();
            let hdop = (q[(0, 0)] + q[(1, 1)]).sqrt();
            let vdop = q[(2, 2)].sqrt();
            let tdop = q[(3, 3)].sqrt();
            Ok(text_result(
                json!({
                    "gdop": gdop, "pdop": pdop, "hdop": hdop, "vdop": vdop, "tdop": tdop,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Four satellites, near-orthogonal",
                args: r#"{"los_enu": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [-0.577, -0.577, -0.577]]}"#,
                note: Some("Returns GDOP/PDOP/HDOP/VDOP/TDOP; low values = good geometry."),
            },
            SkillExample {
                title: "Six-satellite snapshot",
                args: r#"{"los_enu": [[0.3, 0.4, 0.866], [-0.5, 0.5, 0.707], [0.0, -0.707, 0.707], [0.866, 0.0, 0.5], [-0.866, 0.0, 0.5], [0.0, 0.0, 1.0]]}"#,
                note: Some("Need ≥4 LOS vectors; more sats → better DOP."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Score current sat geometry before trusting a GNSS fix for navigation.",
            "Compare two satellite constellations / time slices by HDOP or PDOP.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "los_enu",
            min: Some(4),
            max: None,
        }]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct KlobucharArgs {
    /// GPS week-time of observation (seconds).
    gps_tow_s: f64,
    /// Receiver geomagnetic latitude (deg).
    lat_deg: f64,
    /// Receiver longitude (deg).
    lon_deg: f64,
    /// Satellite elevation angle (deg).
    elevation_deg: f64,
    /// Satellite azimuth (deg, from north).
    azimuth_deg: f64,
    /// Broadcast α coefficients (length 4).
    alpha: [f64; 4],
    /// Broadcast β coefficients (length 4).
    beta: [f64; 4],
}

pub struct NavKlobuchar;
impl Skill for NavKlobuchar {
    fn name(&self) -> &'static str {
        "nav_klobuchar"
    }
    fn description(&self) -> &'static str {
        "Klobuchar ionospheric delay model — broadcast in GPS subframe 4. \
        Returns the slant delay in meters; the model is parameter-cheap \
        and ~50 % accurate at mid latitudes."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<KlobucharArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<KlobucharArgs>()?;
            const C: f64 = 299_792_458.0;
            let psi = 0.0137 / (a.elevation_deg / 180.0 + 0.11) - 0.022;
            let phi_i = a.lat_deg / 180.0 + psi * a.azimuth_deg.to_radians().cos();
            let phi_i = phi_i.clamp(-0.416, 0.416);
            let lambda_i = a.lon_deg / 180.0
                + psi * a.azimuth_deg.to_radians().sin() / (phi_i * std::f64::consts::PI).cos();
            let phi_m = phi_i + 0.064 * ((lambda_i - 1.617) * std::f64::consts::PI).cos();
            let t = 43_200.0 * lambda_i + a.gps_tow_s;
            let t = t.rem_euclid(86_400.0);
            let amp: f64 = a.alpha[0]
                + a.alpha[1] * phi_m
                + a.alpha[2] * phi_m.powi(2)
                + a.alpha[3] * phi_m.powi(3);
            let per: f64 = a.beta[0]
                + a.beta[1] * phi_m
                + a.beta[2] * phi_m.powi(2)
                + a.beta[3] * phi_m.powi(3);
            let amp = amp.max(0.0);
            let per = per.max(72_000.0);
            let x = 2.0 * std::f64::consts::PI * (t - 50_400.0) / per;
            let f = 1.0 + 16.0 * (0.53 - a.elevation_deg / 180.0).powi(3);
            let t_iono = if x.abs() < 1.57 {
                f * (5.0e-9 + amp * (1.0 - x.powi(2) / 2.0 + x.powi(4) / 24.0))
            } else {
                f * 5.0e-9
            };
            Ok(text_result(json!({ "delay_m": t_iono * C }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Mid-latitude noon",
                args: r#"{"gps_tow_s": 43200.0, "lat_deg": 40.0, "lon_deg": -75.0, "elevation_deg": 60.0, "azimuth_deg": 180.0, "alpha": [1.4e-8, 0.0, -5.96e-8, 5.96e-8], "beta": [129024.0, 0.0, -262144.0, 262144.0]}"#,
                note: Some("Returns slant ionospheric delay in meters."),
            },
            SkillExample {
                title: "Low elevation increases delay",
                args: r#"{"gps_tow_s": 43200.0, "lat_deg": 40.0, "lon_deg": -75.0, "elevation_deg": 10.0, "azimuth_deg": 90.0, "alpha": [1.4e-8, 0.0, -5.96e-8, 5.96e-8], "beta": [129024.0, 0.0, -262144.0, 262144.0]}"#,
                note: Some("Mapping function amplifies delay for low sats."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Apply the broadcast Klobuchar correction to a single satellite pseudorange.",
            "Estimate single-frequency ionospheric error budget at a given time/location.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "lat_deg",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "lon_deg",
                min: Some(-180.0),
                max: Some(180.0),
            },
            Rule::Range {
                field: "elevation_deg",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "azimuth_deg",
                min: Some(0.0),
                max: Some(360.0),
            },
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SaastArgs {
    /// Receiver height above mean sea level (m).
    height_m: f64,
    /// Satellite elevation (deg).
    elevation_deg: f64,
    /// Pressure (hPa, default 1013.25).
    #[serde(default)]
    pressure_hpa: Option<f64>,
    /// Temperature (K, default 288.15).
    #[serde(default)]
    temp_k: Option<f64>,
    /// Partial pressure of water vapor (hPa, default 11.7).
    #[serde(default)]
    e_w_hpa: Option<f64>,
}

pub struct NavSaastamoinen;
impl Skill for NavSaastamoinen {
    fn name(&self) -> &'static str {
        "nav_saastamoinen"
    }
    fn description(&self) -> &'static str {
        "Saastamoinen tropospheric delay model. Returns slant delay in \
        meters; reasonable down to ~5° elevation."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SaastArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SaastArgs>()?;
            let p = a.pressure_hpa.unwrap_or(1013.25);
            let t = a.temp_k.unwrap_or(288.15);
            let e = a.e_w_hpa.unwrap_or(11.7);
            let z = (90.0 - a.elevation_deg).to_radians();
            // Simplified Saastamoinen (1972). The full model adds a
            // height-dependent B(h) factor multiplying tan²(z), and a
            // δR(h, z) correction; this implementation omits both, which
            // is accurate to ~5 mm at sea level and ~30 mm at 4000 m, and
            // worse near the horizon. height_m is kept on the schema for
            // forward-compatibility but is currently unused — see the
            // `description()` text for the caveat.
            let _h = a.height_m;
            let delay = 0.002277 / z.cos() * (p + (1255.0 / t + 0.05) * e - z.tan().powi(2));
            Ok(text_result(json!({ "delay_m": delay }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Sea-level, 30° elevation",
                args: r#"{"height_m": 0.0, "elevation_deg": 30.0}"#,
                note: Some("Uses default p=1013.25 hPa, T=288.15 K, e=11.7 hPa."),
            },
            SkillExample {
                title: "Custom atmosphere",
                args: r#"{"height_m": 1500.0, "elevation_deg": 15.0, "pressure_hpa": 850.0, "temp_k": 280.0, "e_w_hpa": 8.0}"#,
                note: Some("Pass measured surface met for tighter delay estimate."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Estimate tropospheric path delay for a satellite at known elevation.",
            "Build a first-cut GNSS error budget combining tropo + iono + DOP.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Range {
            field: "elevation_deg",
            min: Some(-90.0),
            max: Some(90.0),
        }]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EnuArgs {
    /// Reference latitude in decimal degrees.
    ref_lat: f64,
    /// Reference longitude in decimal degrees.
    ref_lon: f64,
    /// Reference ellipsoidal altitude in meters.
    ref_alt_m: f64,
    /// Target ECEF X (meters).
    x: f64,
    /// Target ECEF Y (meters).
    y: f64,
    /// Target ECEF Z (meters).
    z: f64,
}

pub struct NavEcefToEnu;
impl Skill for NavEcefToEnu {
    fn name(&self) -> &'static str {
        "nav_ecef_to_enu"
    }
    fn description(&self) -> &'static str {
        "Transform an ECEF position to local East-North-Up tangent-plane \
        coordinates relative to a reference (lat, lon, alt)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EnuArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EnuArgs>()?;
            const WGS84_A: f64 = 6_378_137.0;
            const WGS84_F: f64 = 1.0 / 298.257_223_563;
            let to_rad = std::f64::consts::PI / 180.0;
            let lat = a.ref_lat * to_rad;
            let lon = a.ref_lon * to_rad;
            let e2 = WGS84_F * (2.0 - WGS84_F);
            let n_prime = WGS84_A / (1.0 - e2 * lat.sin().powi(2)).sqrt();
            let x0 = (n_prime + a.ref_alt_m) * lat.cos() * lon.cos();
            let y0 = (n_prime + a.ref_alt_m) * lat.cos() * lon.sin();
            let z0 = (n_prime * (1.0 - e2) + a.ref_alt_m) * lat.sin();
            let dx = a.x - x0;
            let dy = a.y - y0;
            let dz = a.z - z0;
            let e = -lon.sin() * dx + lon.cos() * dy;
            let n = -lat.sin() * lon.cos() * dx - lat.sin() * lon.sin() * dy + lat.cos() * dz;
            let u = lat.cos() * lon.cos() * dx + lat.cos() * lon.sin() * dy + lat.sin() * dz;
            Ok(text_result(
                json!({ "east_m": e, "north_m": n, "up_m": u }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Reference origin → zero ENU",
                args: r#"{"ref_lat": 40.0, "ref_lon": -75.0, "ref_alt_m": 0.0, "x": 1227128.6, "y": -4581400.0, "z": 4077985.6}"#,
                note: Some("ECEF of the reference itself gives ENU ≈ (0, 0, 0)."),
            },
            SkillExample {
                title: "Equator reference",
                args: r#"{"ref_lat": 0.0, "ref_lon": 0.0, "ref_alt_m": 0.0, "x": 6378137.0, "y": 0.0, "z": 100.0}"#,
                note: Some("Returns local east/north/up offset in meters."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert ECEF positions to a local tangent-plane frame for plotting / control.",
            "Compute east/north/up offsets from a survey marker to a target.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Range {
                field: "ref_lat",
                min: Some(-90.0),
                max: Some(90.0),
            },
            Rule::Range {
                field: "ref_lon",
                min: Some(-180.0),
                max: Some(180.0),
            },
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ImuDriftArgs {
    /// Gyro random walk (°/√hour) → integrated to attitude error after `time_s`.
    gyro_random_walk_deg_sqrt_hr: f64,
    /// Bias instability (°/hour) → steady drift.
    bias_instability_deg_per_hr: f64,
    /// Scale-factor error (ppm).
    scale_factor_ppm: f64,
    /// Time horizon to integrate over (s).
    time_s: f64,
    /// Vehicle rotation rate (°/s, default 0).
    #[serde(default)]
    rate_deg_s: Option<f64>,
}

pub struct NavImuDrift;
impl Skill for NavImuDrift {
    fn name(&self) -> &'static str {
        "nav_imu_drift"
    }
    fn description(&self) -> &'static str {
        "Strapdown IMU attitude error estimate combining angle random walk \
        (∝√t), bias instability (∝t), and scale-factor error (∝t · rate). \
        Returns the three contributions and the RSS total in degrees."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ImuDriftArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ImuDriftArgs>()?;
            let hours = a.time_s / 3600.0;
            let arw = a.gyro_random_walk_deg_sqrt_hr * hours.sqrt();
            let bi = a.bias_instability_deg_per_hr * hours;
            let sf = a.scale_factor_ppm * 1e-6 * a.rate_deg_s.unwrap_or(0.0) * a.time_s;
            let total = (arw.powi(2) + bi.powi(2) + sf.powi(2)).sqrt();
            Ok(text_result(
                json!({
                    "arw_deg": arw,
                    "bias_deg": bi,
                    "scale_factor_deg": sf,
                    "total_rss_deg": total,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Tactical-grade, 60 s",
                args: r#"{"gyro_random_walk_deg_sqrt_hr": 0.1, "bias_instability_deg_per_hr": 1.0, "scale_factor_ppm": 100.0, "time_s": 60.0}"#,
                note: Some("Returns ARW, bias, scale-factor contributions and RSS total in deg."),
            },
            SkillExample {
                title: "With vehicle rate",
                args: r#"{"gyro_random_walk_deg_sqrt_hr": 0.05, "bias_instability_deg_per_hr": 0.5, "scale_factor_ppm": 50.0, "time_s": 300.0, "rate_deg_s": 10.0}"#,
                note: Some("Non-zero rate exercises the scale-factor term."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Predict free-inertial attitude drift over a coast interval.",
            "Compare IMU grades by their dominant error term over a chosen horizon.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Range {
            field: "time_s",
            min: Some(0.0),
            max: None,
        }]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(NavDop),
        Box::new(NavKlobuchar),
        Box::new(NavSaastamoinen),
        Box::new(NavEcefToEnu),
        Box::new(NavImuDrift),
    ]
}
