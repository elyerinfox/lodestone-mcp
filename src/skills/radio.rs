//! Radio-link / RF skills — Friis path loss, Shannon-Hartley capacity, thermal
//! noise, full link budgets, and max-range-for-bandwidth. Pure formulas, no
//! network. Off by default (`[radio].enabled`).

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

pub const TOOL_NAMES: &[&str] = &[
    "radio_friis_path_loss",
    "radio_link_budget",
    "radio_max_range",
    "radio_shannon_capacity",
    "radio_noise_floor",
    "radio_range_for_bandwidth",
];

/// Speed of light, m/s.
const C: f64 = 299_792_458.0;

fn fmt(n: f64) -> String {
    if !n.is_finite() {
        return "n/a".into();
    }
    let s = format!("{n:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn freq_check(hz: f64) -> Result<(), McpError> {
    if !hz.is_finite() || hz <= 0.0 {
        return Err(invalid("frequency_hz must be positive"));
    }
    Ok(())
}

fn dist_check(m: f64) -> Result<(), McpError> {
    if !m.is_finite() || m <= 0.0 {
        return Err(invalid("distance_m must be positive"));
    }
    Ok(())
}

// ----- friis path loss -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FsplArgs {
    /// Frequency in Hz (e.g. 2.4e9 for 2.4 GHz).
    frequency_hz: f64,
    /// Distance in meters.
    distance_m: f64,
}

pub struct RadioFsplPath;
impl Skill for RadioFsplPath {
    fn name(&self) -> &'static str {
        "radio_friis_path_loss"
    }
    fn description(&self) -> &'static str {
        "Friis free-space path loss in dB: FSPL = 20·log10(4·π·d / λ). Pure line-of-sight model, \
        no fading, no terrain. Use as the baseline of a link budget."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FsplArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<FsplArgs>()?;
            freq_check(args.frequency_hz)?;
            dist_check(args.distance_m)?;
            let lambda = C / args.frequency_hz;
            let fspl = 20.0 * (4.0 * std::f64::consts::PI * args.distance_m / lambda).log10();
            Ok(text_result(format!(
                "FSPL at {} Hz over {} m: {} dB  (λ = {} m)",
                fmt(args.frequency_hz),
                fmt(args.distance_m),
                fmt(fspl),
                fmt(lambda)
            )))
        })
    }
}

// ----- noise floor -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NoiseArgs {
    /// Receiver bandwidth in Hz.
    bandwidth_hz: f64,
    /// Receiver noise figure in dB (default 5).
    #[serde(default)]
    noise_figure_db: Option<f64>,
    /// System noise temperature in K (default 290 — room temperature).
    #[serde(default)]
    temperature_k: Option<f64>,
}

pub struct RadioNoiseFloor;
impl Skill for RadioNoiseFloor {
    fn name(&self) -> &'static str {
        "radio_noise_floor"
    }
    fn description(&self) -> &'static str {
        "Thermal noise floor at the receiver: N = kTB + NF. Returns dBm. Default temperature \
        is 290 K (-174 dBm/Hz floor) with a 5 dB receiver noise figure."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoiseArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<NoiseArgs>()?;
            if !args.bandwidth_hz.is_finite() || args.bandwidth_hz <= 0.0 {
                return Err(invalid("bandwidth_hz must be positive"));
            }
            let nf = args.noise_figure_db.unwrap_or(5.0);
            let t = args.temperature_k.unwrap_or(290.0);
            // N₀ (dBm/Hz) = 10·log10(kT/1mW) = 10·log10(1.38e-23 · T) + 30 dB (W→mW)
            let n0_dbm_per_hz = 10.0 * (1.380649e-23 * t).log10() + 30.0;
            let noise_dbm = n0_dbm_per_hz + 10.0 * args.bandwidth_hz.log10() + nf;
            Ok(text_result(format!(
                "Noise floor: {} dBm  (kTB+NF; T={}K, B={} Hz, NF={} dB; N₀ ≈ {} dBm/Hz)",
                fmt(noise_dbm),
                fmt(t),
                fmt(args.bandwidth_hz),
                fmt(nf),
                fmt(n0_dbm_per_hz)
            )))
        })
    }
}

// ----- shannon-hartley -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ShannonArgs {
    /// Channel bandwidth in Hz.
    bandwidth_hz: f64,
    /// Signal-to-noise ratio in dB (S/N).
    snr_db: f64,
}

pub struct RadioShannon;
impl Skill for RadioShannon {
    fn name(&self) -> &'static str {
        "radio_shannon_capacity"
    }
    fn description(&self) -> &'static str {
        "Shannon-Hartley channel capacity: C = B · log2(1 + SNR). Returns bits/s. The hard ceiling \
        on data rate for a given bandwidth and SNR — real systems hit a fraction of this."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ShannonArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<ShannonArgs>()?;
            if !args.bandwidth_hz.is_finite() || args.bandwidth_hz <= 0.0 {
                return Err(invalid("bandwidth_hz must be positive"));
            }
            let snr_lin = 10f64.powf(args.snr_db / 10.0);
            let c = args.bandwidth_hz * (1.0 + snr_lin).log2();
            Ok(text_result(format!(
                "Shannon capacity: {} bit/s  ({} kbit/s, {} Mbit/s) — B={} Hz, SNR={} dB (linear {})",
                fmt(c),
                fmt(c / 1e3),
                fmt(c / 1e6),
                fmt(args.bandwidth_hz),
                fmt(args.snr_db),
                fmt(snr_lin)
            )))
        })
    }
}

// ----- link budget -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LinkBudgetArgs {
    /// Transmitter output power in dBm.
    tx_power_dbm: f64,
    /// Transmit antenna gain in dBi.
    tx_gain_dbi: f64,
    /// Receive antenna gain in dBi.
    rx_gain_dbi: f64,
    /// Other losses (feedline, polarization mismatch, fade margin) in dB. Default 0.
    #[serde(default)]
    other_loss_db: Option<f64>,
    /// Frequency in Hz.
    frequency_hz: f64,
    /// Distance in meters.
    distance_m: f64,
    /// Receiver sensitivity (minimum usable signal) in dBm.
    rx_sensitivity_dbm: f64,
}

pub struct RadioLinkBudget;
impl Skill for RadioLinkBudget {
    fn name(&self) -> &'static str {
        "radio_link_budget"
    }
    fn description(&self) -> &'static str {
        "Free-space link budget: received power = Tx_dBm + Tx_gain + Rx_gain − FSPL − other_loss. \
        Reports received power and link margin vs. the receiver's sensitivity. Positive margin = \
        link closes."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<LinkBudgetArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<LinkBudgetArgs>()?;
            freq_check(args.frequency_hz)?;
            dist_check(args.distance_m)?;
            let lambda = C / args.frequency_hz;
            let fspl = 20.0 * (4.0 * std::f64::consts::PI * args.distance_m / lambda).log10();
            let other = args.other_loss_db.unwrap_or(0.0);
            let rx_dbm = args.tx_power_dbm + args.tx_gain_dbi + args.rx_gain_dbi - fspl - other;
            let margin = rx_dbm - args.rx_sensitivity_dbm;
            let verdict = if margin >= 0.0 {
                "LINK CLOSES"
            } else {
                "below sensitivity — link fails"
            };
            Ok(text_result(format!(
                "Link budget at {} Hz over {} m:\n  Tx: {} dBm + {} dBi  →  EIRP {} dBm\n  Rx: + {} dBi\n  FSPL: −{} dB\n  Other losses: −{} dB\n  Received power: {} dBm\n  Receiver sensitivity: {} dBm\n  Margin: {} dB  ({verdict})",
                fmt(args.frequency_hz),
                fmt(args.distance_m),
                fmt(args.tx_power_dbm),
                fmt(args.tx_gain_dbi),
                fmt(args.tx_power_dbm + args.tx_gain_dbi),
                fmt(args.rx_gain_dbi),
                fmt(fspl),
                fmt(other),
                fmt(rx_dbm),
                fmt(args.rx_sensitivity_dbm),
                fmt(margin),
            )))
        })
    }
}

// ----- max range from a sensitivity -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MaxRangeArgs {
    tx_power_dbm: f64,
    tx_gain_dbi: f64,
    rx_gain_dbi: f64,
    #[serde(default)]
    other_loss_db: Option<f64>,
    frequency_hz: f64,
    /// Receiver sensitivity (minimum usable signal) in dBm.
    rx_sensitivity_dbm: f64,
}

pub struct RadioMaxRange;
impl Skill for RadioMaxRange {
    fn name(&self) -> &'static str {
        "radio_max_range"
    }
    fn description(&self) -> &'static str {
        "Maximum free-space range for a link to close. Solves Friis for d given a known receiver \
        sensitivity. Returns meters and kilometers."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MaxRangeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<MaxRangeArgs>()?;
            freq_check(args.frequency_hz)?;
            let other = args.other_loss_db.unwrap_or(0.0);
            let budget = args.tx_power_dbm + args.tx_gain_dbi + args.rx_gain_dbi
                - args.rx_sensitivity_dbm
                - other;
            // budget = 20·log10(4·π·d / λ)  → d = λ / (4·π) · 10^(budget/20)
            let lambda = C / args.frequency_hz;
            let d = lambda / (4.0 * std::f64::consts::PI) * 10f64.powf(budget / 20.0);
            Ok(text_result(format!(
                "Max FSPL range: {} m  ({} km)  — at {} Hz, link budget {} dB above sensitivity.",
                fmt(d),
                fmt(d / 1000.0),
                fmt(args.frequency_hz),
                fmt(budget)
            )))
        })
    }
}

// ----- range as a function of bandwidth (the user's question) -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RangeForBwArgs {
    tx_power_dbm: f64,
    tx_gain_dbi: f64,
    rx_gain_dbi: f64,
    #[serde(default)]
    other_loss_db: Option<f64>,
    frequency_hz: f64,
    /// Receiver bandwidth in Hz. Wider bandwidth → higher noise floor → shorter range.
    bandwidth_hz: f64,
    /// Required signal-to-noise margin in dB (default 10 — Eb/N0-ish floor for many digital modes).
    #[serde(default)]
    required_snr_db: Option<f64>,
    /// Receiver noise figure in dB (default 5).
    #[serde(default)]
    noise_figure_db: Option<f64>,
    /// System noise temperature in K (default 290).
    #[serde(default)]
    temperature_k: Option<f64>,
}

pub struct RadioRangeForBandwidth;
impl Skill for RadioRangeForBandwidth {
    fn name(&self) -> &'static str {
        "radio_range_for_bandwidth"
    }
    fn description(&self) -> &'static str {
        "How far a radio signal reaches as a function of BANDWIDTH: derives the receiver \
        sensitivity from kTB + NF + required SNR margin, then solves Friis for max range. \
        Doubling bandwidth raises the noise floor by 3 dB and halves the range (roughly). Set \
        `bandwidth_hz`, `frequency_hz`, your Tx power + antenna gains, and a required SNR margin."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RangeForBwArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<RangeForBwArgs>()?;
            freq_check(args.frequency_hz)?;
            if !args.bandwidth_hz.is_finite() || args.bandwidth_hz <= 0.0 {
                return Err(invalid("bandwidth_hz must be positive"));
            }
            let nf = args.noise_figure_db.unwrap_or(5.0);
            let t = args.temperature_k.unwrap_or(290.0);
            let snr = args.required_snr_db.unwrap_or(10.0);
            let other = args.other_loss_db.unwrap_or(0.0);
            let n0 = 10.0 * (1.380649e-23 * t).log10() + 30.0; // dBm/Hz
            let noise_dbm = n0 + 10.0 * args.bandwidth_hz.log10() + nf;
            let sens_dbm = noise_dbm + snr;
            let budget =
                args.tx_power_dbm + args.tx_gain_dbi + args.rx_gain_dbi - sens_dbm - other;
            let lambda = C / args.frequency_hz;
            let d = lambda / (4.0 * std::f64::consts::PI) * 10f64.powf(budget / 20.0);
            // Capacity at the link-edge (SNR = required_snr_db):
            let snr_lin = 10f64.powf(snr / 10.0);
            let cap = args.bandwidth_hz * (1.0 + snr_lin).log2();
            Ok(text_result(format!(
                "Range for {}-Hz bandwidth at {} Hz:\n  noise floor: {} dBm  (kTB + NF; NF={} dB, T={}K)\n  required Rx sensitivity: {} dBm  (noise + {} dB SNR margin)\n  link budget over sensitivity: {} dB\n  → MAX FSPL RANGE: {} m  ({} km)\n  Shannon capacity at this edge: {} bit/s  ({} Mbit/s)\n\nDoubling bandwidth raises the noise floor by 3 dB and roughly halves the range.",
                fmt(args.bandwidth_hz),
                fmt(args.frequency_hz),
                fmt(noise_dbm),
                fmt(nf),
                fmt(t),
                fmt(sens_dbm),
                fmt(snr),
                fmt(budget),
                fmt(d),
                fmt(d / 1000.0),
                fmt(cap),
                fmt(cap / 1e6)
            )))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(RadioFsplPath),
        Box::new(RadioNoiseFloor),
        Box::new(RadioShannon),
        Box::new(RadioLinkBudget),
        Box::new(RadioMaxRange),
        Box::new(RadioRangeForBandwidth),
    ]
}
