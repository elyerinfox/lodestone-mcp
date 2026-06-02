//! Acoustic / underwater propagation skill — sound speed, Snell, transmission
//! loss, sonar equation. Pure math; on by default.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaterSpeedArgs {
    /// Water temperature (°C).
    temp_c: f64,
    /// Salinity (PSU; ≈ 35 in open ocean).
    salinity_psu: f64,
    /// Depth (m).
    depth_m: f64,
}

pub struct AcousticSoundSpeedWater;
impl Skill for AcousticSoundSpeedWater {
    fn name(&self) -> &'static str {
        "acoustic_sound_speed_water"
    }
    fn description(&self) -> &'static str {
        "Sound speed in seawater via the Mackenzie nine-term equation \
        (valid 2..30 °C, 25..40 PSU, 0..8000 m). Returns `c_m_s`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WaterSpeedArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<WaterSpeedArgs>()?;
            let t = a.temp_c;
            let s = a.salinity_psu;
            let d = a.depth_m;
            let c = 1448.96
                + 4.591 * t
                - 5.304e-2 * t.powi(2)
                + 2.374e-4 * t.powi(3)
                + 1.340 * (s - 35.0)
                + 1.630e-2 * d
                + 1.675e-7 * d.powi(2)
                - 1.025e-2 * t * (s - 35.0)
                - 7.139e-13 * t * d.powi(3);
            Ok(text_result(json!({ "c_m_s": c }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AirSpeedArgs {
    /// Air temperature (°C).
    temp_c: f64,
    /// Relative humidity (0..100 %).
    #[serde(default)]
    rh_pct: Option<f64>,
}

pub struct AcousticSoundSpeedAir;
impl Skill for AcousticSoundSpeedAir {
    fn name(&self) -> &'static str {
        "acoustic_sound_speed_air"
    }
    fn description(&self) -> &'static str {
        "Sound speed in air from temperature (and optional humidity). For dry \
        air: c = 20.05 · √(T + 273.15) m/s. Humidity correction adds a tiny \
        bias (typically < 1 m/s)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<AirSpeedArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<AirSpeedArgs>()?;
            let t_k = a.temp_c + 273.15;
            let mut c = 20.05 * t_k.sqrt();
            if let Some(rh) = a.rh_pct {
                if !(0.0..=100.0).contains(&rh) {
                    return Err(invalid("rh_pct must be 0..100"));
                }
                // Approx humidity correction: c += 0.6 * (RH/100).
                c += 0.6 * (rh / 100.0);
            }
            Ok(text_result(json!({ "c_m_s": c }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SnellArgs {
    /// Incident angle from normal (degrees).
    incident_deg: f64,
    /// Sound speed in incident medium.
    c1: f64,
    /// Sound speed in refracting medium.
    c2: f64,
}

pub struct AcousticSnell;
impl Skill for AcousticSnell {
    fn name(&self) -> &'static str {
        "acoustic_snell"
    }
    fn description(&self) -> &'static str {
        "Snell's law refraction angle: sin(θ₂) / c₂ = sin(θ₁) / c₁. Reports \
        `refracted_deg` or sets `total_internal_reflection: true` when no \
        real solution exists."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SnellArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SnellArgs>()?;
            if a.c1 <= 0.0 || a.c2 <= 0.0 {
                return Err(invalid("sound speeds must be > 0"));
            }
            let sin_t2 = a.incident_deg.to_radians().sin() * a.c2 / a.c1;
            if sin_t2.abs() > 1.0 {
                return Ok(text_result(
                    json!({ "total_internal_reflection": true }).to_string(),
                ));
            }
            let theta2 = sin_t2.asin().to_degrees();
            Ok(text_result(json!({ "refracted_deg": theta2 }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TlArgs {
    /// Range (m).
    range_m: f64,
    /// `spherical` (default, deep water far from boundaries) or `cylindrical` (shallow).
    #[serde(default)]
    geometry: Option<String>,
    /// Frequency (kHz) for Thorp absorption.
    frequency_khz: f64,
}

pub struct AcousticTransmissionLoss;
impl Skill for AcousticTransmissionLoss {
    fn name(&self) -> &'static str {
        "acoustic_transmission_loss"
    }
    fn description(&self) -> &'static str {
        "Underwater transmission loss = spreading loss + Thorp absorption \
        loss (dB/km). Spherical spreading TL = 20·log₁₀(R); cylindrical \
        (shallow water) TL = 10·log₁₀(R). Returns `tl_db`, plus the \
        absorption coefficient `alpha_db_per_km`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TlArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<TlArgs>()?;
            if a.range_m <= 0.0 || a.frequency_khz <= 0.0 {
                return Err(invalid("range and frequency must be > 0"));
            }
            let f = a.frequency_khz;
            // Thorp (1967) absorption (dB/km).
            let alpha = 0.11 * f.powi(2) / (1.0 + f.powi(2))
                + 44.0 * f.powi(2) / (4100.0 + f.powi(2))
                + 2.75e-4 * f.powi(2)
                + 0.003;
            let g = a.geometry.unwrap_or_else(|| "spherical".into()).to_lowercase();
            let spreading = match g.as_str() {
                "spherical" => 20.0 * a.range_m.log10(),
                "cylindrical" => 10.0 * a.range_m.log10(),
                other => return Err(invalid(format!("unknown geometry '{other}'"))),
            };
            let absorption = alpha * a.range_m / 1000.0;
            Ok(text_result(
                json!({
                    "tl_db": spreading + absorption,
                    "spreading_db": spreading,
                    "absorption_db": absorption,
                    "alpha_db_per_km": alpha,
                })
                .to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SonarArgs {
    /// Source level (dB re 1 µPa @ 1 m).
    sl_db: f64,
    /// Transmission loss (dB).
    tl_db: f64,
    /// Target strength (dB re 1 m²).
    ts_db: f64,
    /// Noise level (dB re 1 µPa).
    nl_db: f64,
    /// Detection threshold (dB).
    dt_db: f64,
    /// Receive array gain (dB).
    #[serde(default)]
    array_gain_db: Option<f64>,
}

pub struct AcousticSonarEquation;
impl Skill for AcousticSonarEquation {
    fn name(&self) -> &'static str {
        "acoustic_sonar_equation"
    }
    fn description(&self) -> &'static str {
        "Active sonar equation (one-way TL applied twice): SE = SL − 2·TL \
        + TS − (NL − AG) − DT. Positive SE = detection. Returns SE and \
        the components."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SonarArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SonarArgs>()?;
            let ag = a.array_gain_db.unwrap_or(0.0);
            let se = a.sl_db - 2.0 * a.tl_db + a.ts_db - (a.nl_db - ag) - a.dt_db;
            Ok(text_result(
                json!({
                    "signal_excess_db": se,
                    "detection": se > 0.0,
                })
                .to_string(),
            ))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(AcousticSoundSpeedWater),
        Box::new(AcousticSoundSpeedAir),
        Box::new(AcousticSnell),
        Box::new(AcousticTransmissionLoss),
        Box::new(AcousticSonarEquation),
    ]
}
