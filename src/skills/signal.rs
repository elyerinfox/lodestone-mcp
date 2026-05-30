//! Signal-processing skills — FFT, dominant frequencies, RMS, windowing. Pure
//! compute, no network, no file access. Off by default (`[signal].enabled`).
//!
//! Per golden rule 9 each method is its own tool. Input is a raw `Vec<f64>`
//! of real-valued samples; pair with [`crate::skills::wave`] (`wave_samples`)
//! to feed in WAV-decoded audio.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

pub const TOOL_NAMES: &[&str] = &[
    "signal_fft",
    "signal_dominant_frequencies",
    "signal_rms",
    "signal_window",
];

fn validate(values: &[f64]) -> Result<(), McpError> {
    if values.len() < 2 {
        return Err(invalid("need at least 2 samples"));
    }
    if values.iter().any(|v| !v.is_finite()) {
        return Err(invalid("values must all be finite"));
    }
    Ok(())
}

fn fmt_num(x: f64) -> String {
    if !x.is_finite() {
        return "n/a".into();
    }
    let s = format!("{x:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ----- signal_fft -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FftArgs {
    /// Real-valued time-domain samples.
    values: Vec<f64>,
    /// Sample rate in Hz. If given, each bin is labeled with its frequency.
    #[serde(default)]
    sample_rate: Option<f64>,
    /// Max bins to report (default 64, capped at 4096). The bins are the first
    /// `N/2 + 1` after the half-spectrum cut (positive frequencies only).
    #[serde(default)]
    max_bins: Option<u32>,
}

pub struct SignalFft;
impl Skill for SignalFft {
    fn name(&self) -> &'static str {
        "signal_fft"
    }
    fn description(&self) -> &'static str {
        "Real-input FFT: returns the magnitude (and optional frequency in Hz) of each \
        positive-frequency bin. Pass `values` as a numeric array; optional `sample_rate` \
        labels each bin in Hz. Use signal_dominant_frequencies to skip straight to the peaks."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FftArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<FftArgs>()?;
            validate(&args.values)?;
            let n = args.values.len();
            let max_bins = args.max_bins.unwrap_or(64).clamp(1, 4096) as usize;
            let mut buf: Vec<rustfft::num_complex::Complex<f64>> = args
                .values
                .iter()
                .map(|&v| rustfft::num_complex::Complex { re: v, im: 0.0 })
                .collect();
            let mut planner = rustfft::FftPlanner::<f64>::new();
            planner.plan_fft_forward(n).process(&mut buf);
            let half = n / 2 + 1;
            let report_n = half.min(max_bins);
            let mut out = format!("FFT: {n} samples, {half} positive-frequency bins\n");
            if let Some(sr) = args.sample_rate {
                out.push_str(&format!("Sample rate: {} Hz\n", fmt_num(sr)));
            }
            out.push_str("  bin  freq (Hz)        magnitude\n");
            for (i, c) in buf.iter().take(report_n).enumerate() {
                let mag = (c.re * c.re + c.im * c.im).sqrt();
                let freq = args.sample_rate.map(|sr| i as f64 * sr / n as f64);
                let freq_s = freq.map(fmt_num).unwrap_or_else(|| "-".into());
                out.push_str(&format!("  {i:<4} {freq_s:<14}   {}\n", fmt_num(mag)));
            }
            if report_n < half {
                out.push_str(&format!("  … {} more bins truncated\n", half - report_n));
            }
            Ok(text_result(out))
        })
    }
}

// ----- signal_dominant_frequencies -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DominantArgs {
    values: Vec<f64>,
    /// Sample rate in Hz (required to report frequencies).
    sample_rate: f64,
    /// How many peaks to report (default 5, capped at 50).
    #[serde(default)]
    max: Option<u32>,
}

pub struct SignalDominant;
impl Skill for SignalDominant {
    fn name(&self) -> &'static str {
        "signal_dominant_frequencies"
    }
    fn description(&self) -> &'static str {
        "Return the top-N dominant frequencies (Hz) in a signal: runs an FFT and ranks the \
        positive-frequency bins by magnitude. Use signal_fft for the full spectrum."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DominantArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<DominantArgs>()?;
            validate(&args.values)?;
            if !args.sample_rate.is_finite() || args.sample_rate <= 0.0 {
                return Err(invalid("sample_rate must be positive"));
            }
            let n = args.values.len();
            let max = args.max.unwrap_or(5).clamp(1, 50) as usize;
            let mut buf: Vec<rustfft::num_complex::Complex<f64>> = args
                .values
                .iter()
                .map(|&v| rustfft::num_complex::Complex { re: v, im: 0.0 })
                .collect();
            let mut planner = rustfft::FftPlanner::<f64>::new();
            planner.plan_fft_forward(n).process(&mut buf);
            let half = n / 2 + 1;
            // Skip bin 0 (DC) when ranking peaks — it dominates anything offset.
            let mut bins: Vec<(usize, f64)> = (1..half)
                .map(|i| {
                    let c = buf[i];
                    let mag = (c.re * c.re + c.im * c.im).sqrt();
                    (i, mag)
                })
                .collect();
            bins.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let mut out = format!(
                "Top {} dominant frequencies ({} samples @ {} Hz):\n",
                max.min(bins.len()),
                n,
                fmt_num(args.sample_rate)
            );
            out.push_str("  rank  freq (Hz)        magnitude\n");
            for (rank, (bin, mag)) in bins.iter().take(max).enumerate() {
                let freq = *bin as f64 * args.sample_rate / n as f64;
                out.push_str(&format!(
                    "  {:<5} {:<15}  {}\n",
                    rank + 1,
                    fmt_num(freq),
                    fmt_num(*mag)
                ));
            }
            Ok(text_result(out))
        })
    }
}

// ----- signal_rms -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RmsArgs {
    values: Vec<f64>,
}

pub struct SignalRms;
impl Skill for SignalRms {
    fn name(&self) -> &'static str {
        "signal_rms"
    }
    fn description(&self) -> &'static str {
        "Root mean square (RMS) of a signal: sqrt(mean(x^2)). A simple amplitude/energy summary."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RmsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<RmsArgs>()?;
            validate(&args.values)?;
            let n = args.values.len() as f64;
            let sq = args.values.iter().map(|v| v * v).sum::<f64>();
            let rms = (sq / n).sqrt();
            Ok(text_result(format!(
                "RMS: {} ({} samples)",
                fmt_num(rms),
                args.values.len()
            )))
        })
    }
}

// ----- signal_window -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WindowArgs {
    values: Vec<f64>,
    /// One of: hann, hamming, blackman, rectangular.
    kind: String,
}

pub struct SignalWindow;
impl Skill for SignalWindow {
    fn name(&self) -> &'static str {
        "signal_window"
    }
    fn description(&self) -> &'static str {
        "Apply a window function (Hann, Hamming, Blackman, or rectangular = no-op) to a signal \
        prior to FFT. Returns the windowed series (same length). Reduces spectral leakage."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WindowArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<WindowArgs>()?;
            validate(&args.values)?;
            let n = args.values.len();
            let pi = std::f64::consts::PI;
            let win: Vec<f64> = (0..n)
                .map(|i| match args.kind.trim().to_ascii_lowercase().as_str() {
                    "hann" => 0.5 * (1.0 - ((2.0 * pi * i as f64) / (n as f64 - 1.0)).cos()),
                    "hamming" => 0.54 - 0.46 * ((2.0 * pi * i as f64) / (n as f64 - 1.0)).cos(),
                    "blackman" => {
                        let a0 = 0.42;
                        let a1 = 0.5;
                        let a2 = 0.08;
                        a0 - a1 * ((2.0 * pi * i as f64) / (n as f64 - 1.0)).cos()
                            + a2 * ((4.0 * pi * i as f64) / (n as f64 - 1.0)).cos()
                    }
                    "rectangular" | "rect" | "none" => 1.0,
                    _ => f64::NAN,
                })
                .collect();
            if win[0].is_nan() {
                return Err(invalid(
                    "unknown window kind (use hann / hamming / blackman / rectangular)",
                ));
            }
            let out_vals: Vec<f64> = args
                .values
                .iter()
                .zip(win.iter())
                .map(|(v, w)| v * w)
                .collect();
            let preview: Vec<String> = out_vals.iter().take(8).map(|v| fmt_num(*v)).collect();
            Ok(text_result(format!(
                "Applied {} window to {} samples. First {}: [{}]{}",
                args.kind,
                out_vals.len(),
                preview.len(),
                preview.join(", "),
                if out_vals.len() > preview.len() {
                    ", …"
                } else {
                    ""
                }
            )))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(SignalFft),
        Box::new(SignalDominant),
        Box::new(SignalRms),
        Box::new(SignalWindow),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn rms_of_sine_wave() {
        // A unit-amplitude sine has RMS == 1/sqrt(2).
        let n = 1024;
        let y: Vec<f64> = (0..n)
            .map(|t| (2.0 * std::f64::consts::PI * t as f64 / 32.0).sin())
            .collect();
        let sq: f64 = y.iter().map(|v| v * v).sum();
        let rms = (sq / n as f64).sqrt();
        assert!(
            (rms - std::f64::consts::FRAC_1_SQRT_2).abs() < 0.01,
            "got {rms}"
        );
    }
}
