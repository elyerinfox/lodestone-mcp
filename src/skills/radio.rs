//! Radio link-budget skills — composite RF analyses that **chain** the atomic
//! formulas already in [`physics_formula`] (`friis_path_loss`,
//! `thermal_noise_kTB`, `shannon_hartley`) into multi-step engineering
//! workflows. Off by default (`[radio].enabled`).
//!
//! For a single number — just the FSPL, just the noise floor, just the Shannon
//! capacity — call those `physics_formula` entries directly. This module is
//! for the *integrated* analyses that combine them.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

/// Speed of light, m/s.
const C: f64 = 299_792_458.0;

fn fmt(n: f64) -> String {
    if !n.is_finite() {
        return "n/a".into();
    }
    let s = format!("{n:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn fspl_db(frequency_hz: f64, distance_m: f64) -> f64 {
    let lambda = C / frequency_hz;
    20.0 * (4.0 * std::f64::consts::PI * distance_m / lambda).log10()
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
        Reports EIRP, FSPL, received power, and the margin vs. the receiver's sensitivity \
        (positive = link closes). For just the FSPL number use physics_formula \
        name=\"friis_path_loss\"."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<LinkBudgetArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<LinkBudgetArgs>()?;
            freq_check(args.frequency_hz)?;
            dist_check(args.distance_m)?;
            let fspl = fspl_db(args.frequency_hz, args.distance_m);
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

// ----- max range -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MaxRangeArgs {
    /// Transmit power in dBm.
    tx_power_dbm: f64,
    /// Transmit antenna gain in dBi.
    tx_gain_dbi: f64,
    /// Receive antenna gain in dBi.
    rx_gain_dbi: f64,
    /// Other losses in dB (cabling, polarization mismatch, etc.; default 0).
    #[serde(default)]
    other_loss_db: Option<f64>,
    /// Carrier frequency in Hz.
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
        "Maximum free-space range for a link to close. Solves Friis (physics_formula \
        \"friis_path_loss\") for d given a known receiver sensitivity. Returns meters and \
        kilometers. For a derived sensitivity from bandwidth + SNR, use \
        radio_range_for_bandwidth."
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

// ----- range as a function of bandwidth -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RangeForBwArgs {
    /// Transmit power in dBm.
    tx_power_dbm: f64,
    /// Transmit antenna gain in dBi.
    tx_gain_dbi: f64,
    /// Receive antenna gain in dBi.
    rx_gain_dbi: f64,
    /// Other losses in dB (cabling, polarization mismatch, etc.; default 0).
    #[serde(default)]
    other_loss_db: Option<f64>,
    /// Carrier frequency in Hz.
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
        "How far a radio signal reaches as a function of BANDWIDTH: chains \
        physics_formula \"thermal_noise_kTB\" → required Rx sensitivity (noise + SNR margin) → \
        physics_formula \"friis_path_loss\" inverted for d. Doubling bandwidth raises the noise \
        floor by 3 dB and roughly halves the range. Also reports Shannon capacity \
        (physics_formula \"shannon_hartley\") at the link edge."
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
            // Same math as physics_formula "thermal_noise_kTB" with NF folded in.
            let n0 = 10.0 * (1.380649e-23 * t).log10() + 30.0; // dBm/Hz
            let noise_dbm = n0 + 10.0 * args.bandwidth_hz.log10() + nf;
            let sens_dbm = noise_dbm + snr;
            let budget = args.tx_power_dbm + args.tx_gain_dbi + args.rx_gain_dbi - sens_dbm - other;
            let lambda = C / args.frequency_hz;
            let d = lambda / (4.0 * std::f64::consts::PI) * 10f64.powf(budget / 20.0);
            // Shannon capacity at the link-edge (SNR = required_snr_db).
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
        Box::new(RadioLinkBudget),
        Box::new(RadioMaxRange),
        Box::new(RadioRangeForBandwidth),
    ]
}
