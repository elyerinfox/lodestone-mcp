//! Radar equation family — monostatic / bistatic range and SNR, integration
//! gain, pulse-compression processing gain, CFAR thresholds, clutter PDFs.
//! Pure math; on by default.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

const C: f64 = 299_792_458.0;
const K_BOLTZMANN: f64 = 1.380_649e-23;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MonoArgs {
    /// Transmit power (W).
    pt_w: f64,
    /// Antenna gain (linear, NOT dBi).
    gain: f64,
    /// Wavelength (m).
    wavelength_m: f64,
    /// Target radar cross section (m²).
    rcs_m2: f64,
    /// One-way range (m).
    range_m: f64,
    /// Receiver bandwidth (Hz).
    bandwidth_hz: f64,
    /// System noise temperature (K, default 290).
    #[serde(default)]
    noise_temp_k: Option<f64>,
    /// Combined system losses (dB, default 0).
    #[serde(default)]
    losses_db: Option<f64>,
}

pub struct RadarMonostatic;
impl Skill for RadarMonostatic {
    fn name(&self) -> &'static str {
        "radar_monostatic"
    }
    fn description(&self) -> &'static str {
        "Monostatic radar equation: Pr = Pt · G² · λ² · σ / ((4π)³ · R⁴ · L). \
        Returns received signal power, noise power kTBL, single-pulse SNR \
        (linear and dB)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MonoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<MonoArgs>()?;
            if a.pt_w <= 0.0
                || a.gain <= 0.0
                || a.wavelength_m <= 0.0
                || a.rcs_m2 <= 0.0
                || a.range_m <= 0.0
                || a.bandwidth_hz <= 0.0
            {
                return Err(invalid("inputs must be > 0"));
            }
            let t0 = a.noise_temp_k.unwrap_or(290.0);
            let l = 10_f64.powf(a.losses_db.unwrap_or(0.0) / 10.0);
            let pr = a.pt_w * a.gain.powi(2) * a.wavelength_m.powi(2) * a.rcs_m2
                / ((4.0 * std::f64::consts::PI).powi(3) * a.range_m.powi(4) * l);
            let noise = K_BOLTZMANN * t0 * a.bandwidth_hz;
            let snr = pr / noise;
            Ok(text_result(
                json!({
                    "rx_power_w": pr,
                    "noise_w": noise,
                    "snr_linear": snr,
                    "snr_db": 10.0 * snr.log10(),
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "S-band air-surveillance",
                args: r#"{"pt_w": 1000000.0, "gain": 1000.0, "wavelength_m": 0.1, "rcs_m2": 1.0, "range_m": 100000.0, "bandwidth_hz": 1000000.0}"#,
                note: Some("Returns single-pulse SNR; combine with `radar_integration_gain` for the full picture."),
            },
            SkillExample {
                title: "Low SNR with losses",
                args: r#"{"pt_w": 5000.0, "gain": 100.0, "wavelength_m": 0.03, "rcs_m2": 0.1, "range_m": 20000.0, "bandwidth_hz": 5000000.0, "losses_db": 6.0}"#,
                note: Some("`losses_db` aggregates feed / atmospheric / signal-processing losses."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Estimate single-pulse SNR for a monostatic radar geometry.",
            "Sweep `range_m` to find the detection limit for a target RCS.",
            "Trade transmit power vs antenna gain for a fixed SNR target.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BiArgs {
    /// Transmit power (W).
    pt_w: f64,
    /// Transmit antenna gain (linear, NOT dBi).
    gt: f64,
    /// Receive antenna gain (linear, NOT dBi).
    gr: f64,
    /// Wavelength (m).
    wavelength_m: f64,
    /// Bistatic RCS (m²).
    sigma_b_m2: f64,
    /// Tx-to-target range (m).
    rt_m: f64,
    /// Target-to-Rx range (m).
    rr_m: f64,
    /// Receiver bandwidth (Hz).
    bandwidth_hz: f64,
    /// System noise temperature (K, default 290).
    #[serde(default)]
    noise_temp_k: Option<f64>,
    /// Combined system losses (dB, default 0).
    #[serde(default)]
    losses_db: Option<f64>,
}

pub struct RadarBistatic;
impl Skill for RadarBistatic {
    fn name(&self) -> &'static str {
        "radar_bistatic"
    }
    fn description(&self) -> &'static str {
        "Bistatic radar equation: Pr = Pt · Gt · Gr · λ² · σ_b / ((4π)³ · Rt² · Rr² · L). \
        Returns rx power, noise, SNR."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BiArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<BiArgs>()?;
            let t0 = a.noise_temp_k.unwrap_or(290.0);
            let l = 10_f64.powf(a.losses_db.unwrap_or(0.0) / 10.0);
            let pr = a.pt_w * a.gt * a.gr * a.wavelength_m.powi(2) * a.sigma_b_m2
                / ((4.0 * std::f64::consts::PI).powi(3) * a.rt_m.powi(2) * a.rr_m.powi(2) * l);
            let noise = K_BOLTZMANN * t0 * a.bandwidth_hz;
            let snr = pr / noise;
            Ok(text_result(
                json!({
                    "rx_power_w": pr,
                    "noise_w": noise,
                    "snr_linear": snr,
                    "snr_db": 10.0 * snr.log10(),
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Symmetric bistatic geometry",
                args: r#"{"pt_w": 100000.0, "gt": 1000.0, "gr": 1000.0, "wavelength_m": 0.1, "sigma_b_m2": 1.0, "rt_m": 50000.0, "rr_m": 50000.0, "bandwidth_hz": 1000000.0}"#,
                note: Some("With Rt = Rr the equation collapses toward a monostatic equivalent."),
            },
            SkillExample {
                title: "Asymmetric forward-scatter",
                args: r#"{"pt_w": 50000.0, "gt": 500.0, "gr": 500.0, "wavelength_m": 0.03, "sigma_b_m2": 10.0, "rt_m": 100000.0, "rr_m": 5000.0, "bandwidth_hz": 2000000.0, "noise_temp_k": 500.0}"#,
                note: Some(
                    "Short rr_m → strong returns; useful for passive / multistatic studies.",
                ),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Model SNR for passive coherent location or multistatic radar.",
            "Compare bistatic vs monostatic SNR with the same target RCS.",
            "Study forward-scatter geometries where bistatic RCS is enhanced.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IntegrationArgs {
    /// Number of pulses to integrate.
    n: u32,
    /// `coherent` or `noncoherent`.
    method: String,
}

pub struct RadarIntegrationGain;
impl Skill for RadarIntegrationGain {
    fn name(&self) -> &'static str {
        "radar_integration_gain"
    }
    fn description(&self) -> &'static str {
        "Pulse-integration gain in dB. Coherent: 10·log₁₀(N). Non-coherent \
        (square-law detector): approximately 10·log₁₀(N) − L_NC where L_NC \
        is the non-coherent integration loss, approximated here by \
        5·log₁₀(N)/(N+1) (Marcum). Returns gain_db plus an N-dependent \
        non-coherent loss estimate."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IntegrationArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<IntegrationArgs>()?;
            if a.n == 0 {
                return Err(invalid("n must be ≥ 1"));
            }
            let nf = a.n as f64;
            let g = match a.method.to_lowercase().as_str() {
                "coherent" => 10.0 * nf.log10(),
                "noncoherent" => {
                    let nc_loss = 5.0 * nf.log10() / (nf + 1.0);
                    10.0 * nf.log10() - nc_loss
                }
                other => {
                    return Err(invalid(format!(
                        "method must be coherent or noncoherent (got {other})"
                    )))
                }
            };
            Ok(text_result(json!({ "gain_db": g }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Coherent burst of 16 pulses",
                args: r#"{"n": 16, "method": "coherent"}"#,
                note: Some("Returns `gain_db` = 12.04."),
            },
            SkillExample {
                title: "Non-coherent same burst",
                args: r#"{"n": 16, "method": "noncoherent"}"#,
                note: Some("Slightly lower than coherent due to NC integration loss."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Account for pulse integration when budgeting SNR.",
            "Compare coherent vs envelope-detection schemes.",
            "Decide on pulse-train length for a required detection probability.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PulseCompressionArgs {
    /// Pulse duration τ (s).
    pulse_width_s: f64,
    /// Signal bandwidth B (Hz).
    bandwidth_hz: f64,
}

pub struct RadarPulseCompression;
impl Skill for RadarPulseCompression {
    fn name(&self) -> &'static str {
        "radar_pulse_compression_gain"
    }
    fn description(&self) -> &'static str {
        "Pulse-compression processing gain = time-bandwidth product (τ · B). \
        For LFM chirps this equals the compression ratio. Returns linear \
        gain and dB."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PulseCompressionArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<PulseCompressionArgs>()?;
            if a.pulse_width_s <= 0.0 || a.bandwidth_hz <= 0.0 {
                return Err(invalid("pulse width and bandwidth must be > 0"));
            }
            let bt = a.pulse_width_s * a.bandwidth_hz;
            Ok(text_result(
                json!({ "gain_linear": bt, "gain_db": 10.0 * bt.log10() }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Long LFM pulse",
                args: r#"{"pulse_width_s": 0.0001, "bandwidth_hz": 1000000.0}"#,
                note: Some("BT = 100 → 20 dB gain."),
            },
            SkillExample {
                title: "High-resolution chirp",
                args: r#"{"pulse_width_s": 0.00005, "bandwidth_hz": 50000000.0}"#,
                note: Some("BT = 2500 → ~34 dB gain."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Pick a pulse-bandwidth product to hit a desired range resolution.",
            "Convert compressed-pulse SNR back to peak-power-equivalent.",
            "Sanity-check that LFM chirp parameters give enough gain.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CfarArgs {
    /// Number of reference cells (one side).
    n_cells: u32,
    /// Desired probability of false alarm (e.g. 1e-6).
    pfa: f64,
    /// `ca` (cell-averaging) or `os` (order-statistics).
    method: String,
    /// For OS-CFAR, the rank k (1..n_cells).
    #[serde(default)]
    k: Option<u32>,
}

pub struct RadarCfar;
impl Skill for RadarCfar {
    fn name(&self) -> &'static str {
        "radar_cfar_threshold"
    }
    fn description(&self) -> &'static str {
        "CFAR threshold multiplier α for a desired Pfa. CA-CFAR (cell- \
        averaging) over 2N reference cells: α = N · (Pfa^(−1/N) − 1) for \
        Rayleigh / exponential clutter. OS-CFAR uses the kth-order \
        statistic. Returns α (linear). Multiply by the local clutter mean \
        to get the detection threshold."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CfarArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CfarArgs>()?;
            if a.n_cells == 0 {
                return Err(invalid("n_cells must be ≥ 1"));
            }
            if !(0.0..1.0).contains(&a.pfa) {
                return Err(invalid("pfa must be in (0, 1)"));
            }
            let n = a.n_cells as f64;
            let alpha = match a.method.to_lowercase().as_str() {
                "ca" => n * (a.pfa.powf(-1.0 / n) - 1.0),
                "os" => {
                    let k = a.k.ok_or_else(|| invalid("OS-CFAR requires k"))?;
                    if k == 0 || k > a.n_cells {
                        return Err(invalid("OS-CFAR k must be in 1..n_cells"));
                    }
                    // Approximate (Rohling) for moderate N.
                    let kf = k as f64;
                    (kf * (a.pfa.powf(-1.0 / kf) - 1.0)).max(1.0)
                }
                other => return Err(invalid(format!("method must be ca or os (got {other})"))),
            };
            Ok(text_result(json!({ "alpha_linear": alpha }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "CA-CFAR over 32 cells",
                args: r#"{"n_cells": 32, "pfa": 0.000001, "method": "ca"}"#,
                note: Some("Returns the multiplier α applied to local clutter mean."),
            },
            SkillExample {
                title: "OS-CFAR rank 24 of 32",
                args: r#"{"n_cells": 32, "pfa": 0.000001, "method": "os", "k": 24}"#,
                note: Some("Robust to interfering targets in the reference window."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Choose a CFAR threshold for a target probability of false alarm.",
            "Compare CA-CFAR vs OS-CFAR robustness given a clutter scenario.",
            "Calibrate detector logic in a radar signal-processing pipeline.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ClutterArgs {
    /// `rayleigh`, `weibull`, or `k_distribution`.
    distribution: String,
    /// Probability of false alarm.
    pfa: f64,
    /// Weibull shape parameter (k); K-distribution shape (ν).
    #[serde(default)]
    shape: Option<f64>,
}

pub struct RadarClutterThreshold;
impl Skill for RadarClutterThreshold {
    fn name(&self) -> &'static str {
        "radar_clutter_threshold"
    }
    fn description(&self) -> &'static str {
        "Detection threshold scale (relative to clutter mean) for a target Pfa \
        under Rayleigh, Weibull, or K-distribution clutter. Returns the \
        threshold multiplier such that p(clutter > T·μ) = Pfa."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ClutterArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<ClutterArgs>()?;
            if !(0.0..1.0).contains(&a.pfa) {
                return Err(invalid("pfa must be in (0, 1)"));
            }
            let t = match a.distribution.to_lowercase().as_str() {
                "rayleigh" => -a.pfa.ln(),
                "weibull" => {
                    let k = a.shape.ok_or_else(|| invalid("weibull requires shape"))?;
                    if k <= 0.0 {
                        return Err(invalid("weibull shape > 0"));
                    }
                    (-a.pfa.ln()).powf(1.0 / k)
                }
                "k_distribution" => {
                    // Approximate via Rayleigh upper bound × shape correction.
                    let nu = a
                        .shape
                        .ok_or_else(|| invalid("k_distribution requires shape (ν)"))?;
                    if nu <= 0.0 {
                        return Err(invalid("k_distribution shape > 0"));
                    }
                    (-a.pfa.ln()) * (1.0 + 1.0 / nu)
                }
                other => return Err(invalid(format!("unknown distribution '{other}'"))),
            };
            Ok(text_result(
                json!({ "threshold_multiplier": t }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Rayleigh clutter",
                args: r#"{"distribution": "rayleigh", "pfa": 0.0001}"#,
                note: Some("Classical thermal-noise / Rayleigh model."),
            },
            SkillExample {
                title: "Heavy-tailed Weibull",
                args: r#"{"distribution": "weibull", "pfa": 0.0001, "shape": 1.4}"#,
                note: Some("`shape` < 2 means heavier tail than Rayleigh."),
            },
            SkillExample {
                title: "Sea clutter (K-distribution)",
                args: r#"{"distribution": "k_distribution", "pfa": 0.0001, "shape": 0.5}"#,
                note: Some("Common for high-resolution maritime radar."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Choose detection thresholds appropriate for non-Rayleigh sea or ground clutter.",
            "Compare how Pfa changes with clutter heavy-tailedness.",
            "Couple with CFAR α multipliers for end-to-end threshold tuning.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RadarDopplerArgs {
    /// Radar carrier frequency (Hz).
    frequency_hz: f64,
    /// Target radial velocity (m/s, +ve = closing).
    radial_velocity_m_s: f64,
}

pub struct RadarDoppler;
impl Skill for RadarDoppler {
    fn name(&self) -> &'static str {
        "radar_doppler_shift"
    }
    fn description(&self) -> &'static str {
        "Two-way radar Doppler shift Δf = 2 · v_r · f / c."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RadarDopplerArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<RadarDopplerArgs>()?;
            if a.frequency_hz <= 0.0 {
                return Err(invalid("frequency_hz must be > 0"));
            }
            let df = 2.0 * a.radial_velocity_m_s * a.frequency_hz / C;
            Ok(text_result(json!({ "doppler_hz": df }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "X-band closing target",
                args: r#"{"frequency_hz": 10000000000.0, "radial_velocity_m_s": 250.0}"#,
                note: Some("Returns Doppler ≈ 16.7 kHz."),
            },
            SkillExample {
                title: "Receding low-speed",
                args: r#"{"frequency_hz": 24000000000.0, "radial_velocity_m_s": -5.0}"#,
                note: Some("Negative velocity → negative Doppler shift."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert radial velocity to bin index for a PRF-based Doppler filter bank.",
            "Estimate unambiguous-velocity ceiling for a given PRF.",
            "Contrast radar (factor 2) vs one-way radio Doppler.",
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(RadarMonostatic),
        Box::new(RadarBistatic),
        Box::new(RadarIntegrationGain),
        Box::new(RadarPulseCompression),
        Box::new(RadarCfar),
        Box::new(RadarClutterThreshold),
        Box::new(RadarDoppler),
    ]
}
