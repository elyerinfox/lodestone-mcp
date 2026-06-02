//! RF link-engineering skill — propagation models, Doppler, polarization,
//! Fresnel-zone geometry, knife-edge diffraction, and a full Friis-with-
//! noise link calculator. Pure math; on by default. Complements the
//! existing `radio_*` family.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

const C_M_PER_S: f64 = 299_792_458.0;

fn freq_check(f_hz: f64) -> std::result::Result<(), McpError> {
    if f_hz <= 0.0 {
        return Err(invalid("frequency_hz must be > 0"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Path-loss models
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TwoRayArgs {
    frequency_hz: f64,
    /// Transmitter height above ground (m).
    tx_height_m: f64,
    /// Receiver height above ground (m).
    rx_height_m: f64,
    distance_m: f64,
}

pub struct RfTwoRayPathLoss;
impl Skill for RfTwoRayPathLoss {
    fn name(&self) -> &'static str {
        "rf_two_ray_path_loss"
    }
    fn description(&self) -> &'static str {
        "Two-ray ground-reflection path-loss model (line-of-sight + perfectly \
        reflected ground ray). Closed-form approximation valid when \
        d ≫ √(h_tx · h_rx): L = 40·log₁₀(d) − 20·log₁₀(h_tx · h_rx) dB. \
        Frequency-independent in this asymptotic form."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<TwoRayArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<TwoRayArgs>()?;
            freq_check(a.frequency_hz)?;
            if a.distance_m <= 0.0 || a.tx_height_m <= 0.0 || a.rx_height_m <= 0.0 {
                return Err(invalid("distance and antenna heights must be > 0"));
            }
            let loss_db =
                40.0 * a.distance_m.log10() - 20.0 * (a.tx_height_m * a.rx_height_m).log10();
            Ok(text_result(json!({ "path_loss_db": loss_db }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HataArgs {
    /// 150 MHz ≤ f ≤ 1500 MHz for Hata; up to 2000 MHz for COST-231.
    frequency_mhz: f64,
    /// Base station antenna height (30..200 m).
    bs_height_m: f64,
    /// Mobile antenna height (1..10 m).
    mobile_height_m: f64,
    /// Distance (km, 1..20).
    distance_km: f64,
    /// One of `urban_large`, `urban_small`, `suburban`, `open`.
    environment: String,
}

pub struct RfHataPathLoss;
impl Skill for RfHataPathLoss {
    fn name(&self) -> &'static str {
        "rf_hata_path_loss"
    }
    fn description(&self) -> &'static str {
        "Okumura-Hata path-loss model (150–1500 MHz, urban / suburban / open). \
        Returns `path_loss_db`. Empirical; calibrated for cellular UHF/VHF \
        land-mobile."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HataArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<HataArgs>()?;
            if !(150.0..=1500.0).contains(&a.frequency_mhz) {
                return Err(invalid("Hata: 150 MHz ≤ frequency ≤ 1500 MHz"));
            }
            if !(30.0..=200.0).contains(&a.bs_height_m) {
                return Err(invalid("BS height must be 30..200 m"));
            }
            if !(1.0..=10.0).contains(&a.mobile_height_m) {
                return Err(invalid("mobile height must be 1..10 m"));
            }
            if !(1.0..=100.0).contains(&a.distance_km) {
                return Err(invalid("distance must be 1..100 km"));
            }
            let f = a.frequency_mhz;
            let hb = a.bs_height_m;
            let hm = a.mobile_height_m;
            let d = a.distance_km;

            // Mobile antenna correction.
            let a_hm = match a.environment.to_lowercase().as_str() {
                "urban_large" => {
                    if f >= 200.0 {
                        3.2 * (11.75 * hm).log10().powi(2) - 4.97
                    } else {
                        8.29 * (1.54 * hm).log10().powi(2) - 1.1
                    }
                }
                "urban_small" | "suburban" | "open" => {
                    (1.1 * f.log10() - 0.7) * hm - (1.56 * f.log10() - 0.8)
                }
                other => return Err(invalid(format!("unknown environment '{other}'"))),
            };

            let l_urban = 69.55 + 26.16 * f.log10() - 13.82 * hb.log10() - a_hm
                + (44.9 - 6.55 * hb.log10()) * d.log10();
            let l = match a.environment.to_lowercase().as_str() {
                "urban_large" | "urban_small" => l_urban,
                "suburban" => l_urban - 2.0 * (f / 28.0).log10().powi(2) - 5.4,
                "open" => l_urban - 4.78 * f.log10().powi(2) + 18.33 * f.log10() - 40.94,
                _ => unreachable!(),
            };
            Ok(text_result(json!({ "path_loss_db": l }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct Cost231Args {
    frequency_mhz: f64,
    bs_height_m: f64,
    mobile_height_m: f64,
    distance_km: f64,
    /// `medium_small_cities` (default 0 dB add) or `metro_large` (3 dB add).
    environment: String,
}

pub struct RfCost231PathLoss;
impl Skill for RfCost231PathLoss {
    fn name(&self) -> &'static str {
        "rf_cost231_path_loss"
    }
    fn description(&self) -> &'static str {
        "COST-231 extension of Hata to 1500–2000 MHz (PCS / GSM-1900). \
        Returns `path_loss_db`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<Cost231Args>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<Cost231Args>()?;
            if !(1500.0..=2000.0).contains(&a.frequency_mhz) {
                return Err(invalid("COST-231: 1500 MHz ≤ frequency ≤ 2000 MHz"));
            }
            let f = a.frequency_mhz;
            let hb = a.bs_height_m;
            let hm = a.mobile_height_m;
            let d = a.distance_km;
            let a_hm = (1.1 * f.log10() - 0.7) * hm - (1.56 * f.log10() - 0.8);
            let cm = match a.environment.to_lowercase().as_str() {
                "metro_large" => 3.0,
                "medium_small_cities" => 0.0,
                other => return Err(invalid(format!("unknown environment '{other}'"))),
            };
            let l = 46.3 + 33.9 * f.log10() - 13.82 * hb.log10() - a_hm
                + (44.9 - 6.55 * hb.log10()) * d.log10()
                + cm;
            Ok(text_result(json!({ "path_loss_db": l }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EgliArgs {
    frequency_mhz: f64,
    tx_height_m: f64,
    rx_height_m: f64,
    distance_km: f64,
}

pub struct RfEgliPathLoss;
impl Skill for RfEgliPathLoss {
    fn name(&self) -> &'static str {
        "rf_egli_path_loss"
    }
    fn description(&self) -> &'static str {
        "Egli's terrain-irregularity path-loss model. Quick estimate over \
        gently-rolling terrain for VHF/UHF point-to-point. Returns \
        `path_loss_db`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EgliArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<EgliArgs>()?;
            if a.frequency_mhz <= 0.0
                || a.tx_height_m <= 0.0
                || a.rx_height_m <= 0.0
                || a.distance_km <= 0.0
            {
                return Err(invalid("all inputs must be > 0"));
            }
            let d_m = a.distance_km * 1000.0;
            let beta = (40.0 / a.frequency_mhz).powi(2);
            let path_gain = (a.tx_height_m * a.rx_height_m).powi(2) / d_m.powi(4) * beta;
            let loss_db = -10.0 * path_gain.log10();
            Ok(text_result(json!({ "path_loss_db": loss_db }).to_string()))
        })
    }
}

// ---------------------------------------------------------------------------
// ITU-R atmospheric / rain absorption
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct Itup676Args {
    /// Frequency in GHz (1..1000).
    frequency_ghz: f64,
    /// Atmospheric pressure (hPa, default 1013.25).
    #[serde(default)]
    pressure_hpa: Option<f64>,
    /// Air temperature (°C, default 15).
    #[serde(default)]
    temp_c: Option<f64>,
    /// Water-vapor density (g/m³, default 7.5).
    #[serde(default)]
    water_vapor_g_m3: Option<f64>,
}

pub struct RfItuP676Absorption;
impl Skill for RfItuP676Absorption {
    fn name(&self) -> &'static str {
        "rf_itu_p676_absorption"
    }
    fn description(&self) -> &'static str {
        "Approximate atmospheric specific attenuation γ (dB/km) at the given \
        frequency, pressure, temperature, and water vapor density — a \
        simplified ITU-R P.676 implementation suitable for first-cut link \
        budgets up to ~100 GHz. Returns `gamma_db_per_km` (dry+wet sum) \
        plus the two components. For peak accuracy near the resonance \
        lines (e.g. 22 GHz, 60 GHz) use the full line-by-line P.676 model."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<Itup676Args>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<Itup676Args>()?;
            if !(1.0..=1000.0).contains(&a.frequency_ghz) {
                return Err(invalid("frequency must be 1..1000 GHz"));
            }
            let f = a.frequency_ghz;
            let p = a.pressure_hpa.unwrap_or(1013.25);
            let t = a.temp_c.unwrap_or(15.0) + 273.15;
            let rho = a.water_vapor_g_m3.unwrap_or(7.5);

            // Dry-air attenuation simplified (Liebe approximation, not full P.676).
            let theta = 300.0 / t;
            let gamma_o = (7.27 * theta / (f.powi(2) + 0.351 * theta.powi(2))
                + 7.5 / ((f - 60.0).powi(2) + 15.0))
                * f.powi(2)
                * p
                * 1e-3
                * theta.powi(2);

            // Water-vapor attenuation simplified.
            let gamma_w = (3.27e-2 * theta
                + 0.067 * theta.powi(3)
                + 7.3 / ((f - 22.235).powi(2) + 6.6)
                + 11.4 / ((f - 183.31).powi(2) + 5.0)
                + 0.07 / ((f - 325.153).powi(2) + 1.5))
                * f.powi(2)
                * rho
                * 1e-4
                * theta.powf(2.5);

            let gamma = gamma_o.abs() + gamma_w.abs();
            Ok(text_result(
                json!({
                    "gamma_db_per_km": gamma,
                    "gamma_dry_db_per_km": gamma_o.abs(),
                    "gamma_wet_db_per_km": gamma_w.abs(),
                })
                .to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct Itup838Args {
    /// Frequency in GHz (1..1000).
    frequency_ghz: f64,
    /// Rain rate in mm/hour.
    rain_rate_mm_h: f64,
    /// Polarization: `horizontal`, `vertical`, `circular`.
    polarization: String,
}

pub struct RfItuP838Rain;
impl Skill for RfItuP838Rain {
    fn name(&self) -> &'static str {
        "rf_itu_p838_rain"
    }
    fn description(&self) -> &'static str {
        "Specific rain attenuation γ_R = k · R^α (dB/km) per ITU-R P.838. \
        Coefficients k, α are frequency- and polarization-dependent; here we \
        use the standard fit valid 1–1000 GHz. Returns `gamma_db_per_km` \
        and `(k, alpha)` actually applied."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<Itup838Args>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<Itup838Args>()?;
            if a.rain_rate_mm_h < 0.0 {
                return Err(invalid("rain_rate_mm_h must be ≥ 0"));
            }
            let f = a.frequency_ghz;
            // Compact log-log fits (Olsen-Rogers-Hodge family); standard reference for P.838.
            let (kh, ah) = (
                10_f64.powf(-5.339_05 + 1.518_3 * (f.log10()) - 0.190_19 * (f.log10()).powi(2)),
                1.282_5 - 0.034_3 * (f.log10()),
            );
            let (kv, av) = (
                10_f64.powf(-5.387_15 + 1.581_84 * (f.log10()) - 0.221_5 * (f.log10()).powi(2)),
                1.273_1 - 0.038_38 * (f.log10()),
            );
            let (k, alpha) = match a.polarization.to_lowercase().as_str() {
                "horizontal" | "h" => (kh, ah),
                "vertical" | "v" => (kv, av),
                "circular" => ((kh + kv) / 2.0, (kh * ah + kv * av) / (kh + kv)),
                other => return Err(invalid(format!("unknown polarization '{other}'"))),
            };
            let gamma = k * a.rain_rate_mm_h.powf(alpha);
            Ok(text_result(
                json!({
                    "gamma_db_per_km": gamma,
                    "k": k,
                    "alpha": alpha,
                })
                .to_string(),
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Doppler / polarization / Fresnel / knife-edge
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DopplerArgs {
    frequency_hz: f64,
    /// Relative line-of-sight velocity (m/s, +ve = closing).
    velocity_m_s: f64,
}

pub struct RfDopplerShift;
impl Skill for RfDopplerShift {
    fn name(&self) -> &'static str {
        "rf_doppler_shift"
    }
    fn description(&self) -> &'static str {
        "Classical Doppler shift Δf = v_los · f / c. Use the line-of-sight \
        component of relative velocity (+ = closing). Returns `doppler_hz` \
        and the apparent (shifted) frequency `apparent_hz`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DopplerArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<DopplerArgs>()?;
            freq_check(a.frequency_hz)?;
            let doppler = a.velocity_m_s * a.frequency_hz / C_M_PER_S;
            Ok(text_result(
                json!({
                    "doppler_hz": doppler,
                    "apparent_hz": a.frequency_hz + doppler,
                })
                .to_string(),
            ))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PolarizationArgs {
    /// Tx polarization: `linear_h`, `linear_v`, `linear_at_deg`, `rhcp`, `lhcp`.
    tx: String,
    rx: String,
    /// When using `linear_at_deg` for tx or rx, the orientation angle (degrees).
    #[serde(default)]
    tx_angle_deg: Option<f64>,
    #[serde(default)]
    rx_angle_deg: Option<f64>,
}

pub struct RfPolarizationLoss;
impl Skill for RfPolarizationLoss {
    fn name(&self) -> &'static str {
        "rf_polarization_loss"
    }
    fn description(&self) -> &'static str {
        "Polarization mismatch loss between Tx and Rx antennas. Linear-linear: \
        20·log₁₀|cos θ| where θ is the angle between polarizations. \
        Circular-circular same sense: 0 dB; opposite sense: ∞. Linear-circular: \
        3 dB. Returns `loss_db`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PolarizationArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PolarizationArgs>()?;
            let tx_norm = pol_to_axis(&a.tx, a.tx_angle_deg)?;
            let rx_norm = pol_to_axis(&a.rx, a.rx_angle_deg)?;
            let loss_db = match (tx_norm, rx_norm) {
                (Some(t_axis), Some(r_axis)) => {
                    let diff = (t_axis - r_axis).abs();
                    let c = diff.to_radians().cos().abs();
                    if c < 1e-9 {
                        100.0
                    } else {
                        -20.0 * c.log10()
                    }
                }
                (None, None) => {
                    // Both circular; same sense check.
                    if a.tx.to_lowercase() == a.rx.to_lowercase() {
                        0.0
                    } else {
                        100.0
                    }
                }
                _ => 3.0, // linear ↔ circular
            };
            Ok(text_result(json!({ "loss_db": loss_db }).to_string()))
        })
    }
}

fn pol_to_axis(name: &str, angle: Option<f64>) -> std::result::Result<Option<f64>, McpError> {
    match name.to_lowercase().as_str() {
        "linear_h" | "h" => Ok(Some(0.0)),
        "linear_v" | "v" => Ok(Some(90.0)),
        "linear_at_deg" => {
            Ok(Some(angle.ok_or_else(|| {
                invalid("linear_at_deg requires angle_deg")
            })?))
        }
        "rhcp" | "lhcp" => Ok(None),
        other => Err(invalid(format!("unknown polarization '{other}'"))),
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FresnelArgs {
    frequency_hz: f64,
    /// Total path length (m).
    distance_m: f64,
    /// Distance from Tx to the obstruction point (m).
    distance_to_obstruction_m: f64,
    /// Fresnel-zone number (1 = first zone, default 1).
    #[serde(default)]
    n: Option<u8>,
}

pub struct RfFresnelZoneRadius;
impl Skill for RfFresnelZoneRadius {
    fn name(&self) -> &'static str {
        "rf_fresnel_zone_radius"
    }
    fn description(&self) -> &'static str {
        "Fresnel-zone radius F_n = √(n · λ · d1 · d2 / d) at a point along \
        the path, where d1 is the Tx-to-point distance, d2 = d − d1, and d \
        is the total path length. The first Fresnel zone (n=1) is the \
        practical clearance criterion (60 % typically required)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FresnelArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<FresnelArgs>()?;
            freq_check(a.frequency_hz)?;
            if a.distance_m <= 0.0
                || a.distance_to_obstruction_m <= 0.0
                || a.distance_to_obstruction_m >= a.distance_m
            {
                return Err(invalid("require 0 < d1 < d"));
            }
            let lambda = C_M_PER_S / a.frequency_hz;
            let d1 = a.distance_to_obstruction_m;
            let d2 = a.distance_m - d1;
            let n = a.n.unwrap_or(1) as f64;
            let f = (n * lambda * d1 * d2 / a.distance_m).sqrt();
            Ok(text_result(json!({ "fresnel_radius_m": f }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct KnifeEdgeArgs {
    frequency_hz: f64,
    /// Distance from Tx to the knife edge (m).
    d1_m: f64,
    /// Distance from knife edge to Rx (m).
    d2_m: f64,
    /// Edge clearance height above the line of sight (m; negative = obstructed).
    h_m: f64,
}

pub struct RfKnifeEdgeDiffraction;
impl Skill for RfKnifeEdgeDiffraction {
    fn name(&self) -> &'static str {
        "rf_knife_edge_diffraction"
    }
    fn description(&self) -> &'static str {
        "Single-edge knife-edge diffraction loss via the Fresnel-Kirchhoff \
        parameter v = h · √(2/λ · (1/d1 + 1/d2)). Uses Lee's approximation. \
        Returns `loss_db` (positive = additional loss above free space) and \
        the parameter `v`. `h` is the clearance above the line of sight \
        (positive = clear, negative = obstructing)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<KnifeEdgeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<KnifeEdgeArgs>()?;
            freq_check(a.frequency_hz)?;
            if a.d1_m <= 0.0 || a.d2_m <= 0.0 {
                return Err(invalid("d1 and d2 must be > 0"));
            }
            let lambda = C_M_PER_S / a.frequency_hz;
            let v = a.h_m * (2.0 / lambda * (1.0 / a.d1_m + 1.0 / a.d2_m)).sqrt();
            // Lee's approximation for J(v).
            let loss = if v < -1.0 {
                0.0
            } else if v <= 0.0 {
                20.0 * (0.5 - 0.62 * v).log10()
            } else if v <= 1.0 {
                20.0 * (0.5 * (-0.95 * v).exp()).log10()
            } else if v <= 2.4 {
                20.0 * (0.4 - (0.1184 - (0.38 - 0.1 * v).powi(2)).sqrt()).log10()
            } else {
                20.0 * (0.225 / v).log10()
            };
            Ok(text_result(json!({ "loss_db": -loss, "v": v }).to_string()))
        })
    }
}

// ---------------------------------------------------------------------------
// Friis-with-noise link calculator
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LinkArgs {
    frequency_hz: f64,
    distance_m: f64,
    tx_power_dbm: f64,
    tx_gain_dbi: f64,
    rx_gain_dbi: f64,
    /// Additional path losses (atmospheric, rain, polarization, etc.) in dB.
    #[serde(default)]
    extra_loss_db: Option<f64>,
    /// Receiver bandwidth in Hz.
    bandwidth_hz: f64,
    /// System noise figure in dB (default 3).
    #[serde(default)]
    noise_figure_db: Option<f64>,
    /// Required Eb/N0 (or SNR) in dB (default 10).
    #[serde(default)]
    required_snr_db: Option<f64>,
}

pub struct RfFriisWithNoise;
impl Skill for RfFriisWithNoise {
    fn name(&self) -> &'static str {
        "rf_friis_with_noise"
    }
    fn description(&self) -> &'static str {
        "Full link calculation: free-space path loss + extra losses → Rx \
        power, thermal noise power (kTBF), receive SNR, and margin vs. \
        required SNR. Returns each component for sanity-checking."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<LinkArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<LinkArgs>()?;
            freq_check(a.frequency_hz)?;
            if a.distance_m <= 0.0 || a.bandwidth_hz <= 0.0 {
                return Err(invalid("distance and bandwidth must be > 0"));
            }
            let lambda = C_M_PER_S / a.frequency_hz;
            let fspl_db = 20.0 * (4.0 * std::f64::consts::PI * a.distance_m / lambda).log10();
            let extra = a.extra_loss_db.unwrap_or(0.0);
            let nf = a.noise_figure_db.unwrap_or(3.0);
            let req = a.required_snr_db.unwrap_or(10.0);
            let rx_dbm = a.tx_power_dbm + a.tx_gain_dbi + a.rx_gain_dbi - fspl_db - extra;
            // Thermal noise: -174 + 10log10(B) + NF (dBm).
            let noise_dbm = -174.0 + 10.0 * a.bandwidth_hz.log10() + nf;
            let snr_db = rx_dbm - noise_dbm;
            let margin_db = snr_db - req;
            Ok(text_result(
                json!({
                    "free_space_loss_db": fspl_db,
                    "rx_power_dbm": rx_dbm,
                    "noise_floor_dbm": noise_dbm,
                    "snr_db": snr_db,
                    "margin_db": margin_db,
                })
                .to_string(),
            ))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(RfTwoRayPathLoss),
        Box::new(RfHataPathLoss),
        Box::new(RfCost231PathLoss),
        Box::new(RfEgliPathLoss),
        Box::new(RfItuP676Absorption),
        Box::new(RfItuP838Rain),
        Box::new(RfDopplerShift),
        Box::new(RfPolarizationLoss),
        Box::new(RfFresnelZoneRadius),
        Box::new(RfKnifeEdgeDiffraction),
        Box::new(RfFriisWithNoise),
    ]
}
