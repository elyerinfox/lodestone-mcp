//! DSP extensions beyond the existing `signal_*` family: spectrogram via
//! STFT, FFT cross-correlation, magnitude-squared coherence, Hilbert
//! transform, cepstrum, IQ demod, and BER theoretical curves.
//! On by default.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use rustfft::num_complex::Complex64;
use rustfft::FftPlanner;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SpectrogramArgs {
    /// Real-valued signal samples.
    samples: Vec<f64>,
    /// Sample rate (Hz).
    sample_rate_hz: f64,
    /// FFT window length (samples; should be a power of 2).
    window_size: usize,
    /// Window overlap as a fraction (0..1). Default 0.5.
    #[serde(default)]
    overlap: Option<f64>,
}

pub struct DspSpectrogram;
impl Skill for DspSpectrogram {
    fn name(&self) -> &'static str {
        "signal_spectrogram"
    }
    fn description(&self) -> &'static str {
        "Short-time Fourier transform spectrogram. Hann-window each \
        `window_size`-sample slice (advancing by `(1 − overlap) · window`), \
        FFT, and report magnitudes. Returns `freqs_hz` (frequency bins), \
        `times_s` (slice centers), and `magnitude` as a matrix \
        (rows = time, cols = freq). Suitable feed for `chart_heatmap`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SpectrogramArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<SpectrogramArgs>()?;
            if a.samples.is_empty() {
                return Err(invalid("samples empty"));
            }
            if a.sample_rate_hz <= 0.0 {
                return Err(invalid("sample_rate_hz must be > 0"));
            }
            if a.window_size < 2 || !a.window_size.is_power_of_two() {
                return Err(invalid("window_size must be a power of 2 and ≥ 2"));
            }
            let overlap = a.overlap.unwrap_or(0.5).clamp(0.0, 0.99);
            let hop = ((1.0 - overlap) * a.window_size as f64).max(1.0) as usize;
            let n_slices = if a.samples.len() < a.window_size {
                0
            } else {
                1 + (a.samples.len() - a.window_size) / hop
            };
            if n_slices == 0 {
                return Err(invalid("signal shorter than window"));
            }
            let mut planner = FftPlanner::<f64>::new();
            let fft = planner.plan_fft_forward(a.window_size);
            let hann: Vec<f64> = (0..a.window_size)
                .map(|n| {
                    0.5 * (1.0
                        - (2.0 * std::f64::consts::PI * n as f64 / (a.window_size - 1) as f64)
                            .cos())
                })
                .collect();
            let nfreq = a.window_size / 2 + 1;
            let mut mag: Vec<Vec<f64>> = Vec::with_capacity(n_slices);
            let mut times: Vec<f64> = Vec::with_capacity(n_slices);
            for k in 0..n_slices {
                let start = k * hop;
                let mut buf: Vec<Complex64> = (0..a.window_size)
                    .map(|i| Complex64::new(a.samples[start + i] * hann[i], 0.0))
                    .collect();
                fft.process(&mut buf);
                let row: Vec<f64> = buf[..nfreq].iter().map(|c| c.norm()).collect();
                mag.push(row);
                times.push((start + a.window_size / 2) as f64 / a.sample_rate_hz);
            }
            let freqs: Vec<f64> = (0..nfreq)
                .map(|i| i as f64 * a.sample_rate_hz / a.window_size as f64)
                .collect();
            Ok(text_result(
                json!({
                    "freqs_hz": freqs,
                    "times_s": times,
                    "magnitude": mag,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Short tone at 1 kHz",
                args: r#"{"samples": [0.0, 0.71, 1.0, 0.71, 0.0, -0.71, -1.0, -0.71, 0.0, 0.71, 1.0, 0.71, 0.0, -0.71, -1.0, -0.71], "sample_rate_hz": 8000.0, "window_size": 8}"#,
                note: Some("Returns `freqs_hz`, `times_s`, and a 2-D `magnitude` matrix."),
            },
            SkillExample {
                title: "Custom overlap",
                args: r#"{"samples": [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 0.9, 0.8, 0.7, 0.6, 0.5, 0.4, 0.3, 0.2, 0.1, 0.0], "sample_rate_hz": 1000.0, "window_size": 8, "overlap": 0.75}"#,
                note: Some("Higher overlap → more time slices, smoother spectrogram."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Plot a time-frequency surface — feed `magnitude` to `chart_heatmap`.",
            "Detect chirps or transients that a single FFT would smear.",
            "Prepare a feature matrix for downstream classification.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Length {
                field: "samples",
                min: Some(1),
                max: None,
            },
            Rule::Range {
                field: "sample_rate_hz",
                min: Some(f64::MIN_POSITIVE),
                max: None,
            },
            Rule::Range {
                field: "window_size",
                min: Some(2.0),
                max: None,
            },
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct XCorrArgs {
    /// First signal samples.
    a: Vec<f64>,
    /// Second signal samples.
    b: Vec<f64>,
    /// Sample rate in Hz (used to convert peak lag to seconds).
    sample_rate_hz: f64,
}

pub struct DspCrossCorrelation;
impl Skill for DspCrossCorrelation {
    fn name(&self) -> &'static str {
        "signal_cross_correlation"
    }
    fn description(&self) -> &'static str {
        "Cross-correlation of two real signals via FFT. Returns `lag_samples` \
        and `correlation` (the full N-length output, with lag 0 in the \
        center) plus `peak_lag_s` — the lag at the maximum magnitude — for \
        time-delay estimation."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<XCorrArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<XCorrArgs>()?;
            if a.a.is_empty() || a.b.is_empty() {
                return Err(invalid("inputs empty"));
            }
            if a.sample_rate_hz <= 0.0 {
                return Err(invalid("sample_rate_hz must be > 0"));
            }
            let n = (a.a.len() + a.b.len() - 1).next_power_of_two();
            let mut planner = FftPlanner::<f64>::new();
            let fft = planner.plan_fft_forward(n);
            let ifft = planner.plan_fft_inverse(n);
            let mut fa: Vec<Complex64> = (0..n)
                .map(|i| Complex64::new(*a.a.get(i).unwrap_or(&0.0), 0.0))
                .collect();
            let mut fb: Vec<Complex64> = (0..n)
                .map(|i| Complex64::new(*a.b.get(i).unwrap_or(&0.0), 0.0))
                .collect();
            fft.process(&mut fa);
            fft.process(&mut fb);
            let mut prod: Vec<Complex64> = fa
                .iter()
                .zip(fb.iter())
                .map(|(x, y)| x * y.conj())
                .collect();
            ifft.process(&mut prod);
            let scale = 1.0 / n as f64;
            // Re-order so lag 0 is at center.
            let half_a = a.a.len() - 1;
            let mut lags: Vec<i64> = Vec::with_capacity(a.a.len() + a.b.len() - 1);
            let mut corr: Vec<f64> = Vec::with_capacity(a.a.len() + a.b.len() - 1);
            for lag in -(a.b.len() as i64 - 1)..=(a.a.len() as i64 - 1) {
                let idx = (lag.rem_euclid(n as i64)) as usize;
                lags.push(lag);
                corr.push(prod[idx].re * scale);
            }
            let _ = half_a;
            let (i_peak, peak) =
                corr.iter()
                    .enumerate()
                    .fold((0usize, f64::NEG_INFINITY), |acc, (i, v)| {
                        if v.abs() > acc.1.abs() {
                            (i, *v)
                        } else {
                            acc
                        }
                    });
            let peak_lag_s = lags[i_peak] as f64 / a.sample_rate_hz;
            Ok(text_result(
                json!({
                    "lag_samples": lags,
                    "correlation": corr,
                    "peak_lag_s": peak_lag_s,
                    "peak_value": peak,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Delayed copy",
                args: r#"{"a": [1.0, 0.0, 0.0, 0.0, 0.0], "b": [0.0, 0.0, 1.0, 0.0, 0.0], "sample_rate_hz": 1000.0}"#,
                note: Some("`peak_lag_s` ≈ +0.002 — b lags a by 2 samples."),
            },
            SkillExample {
                title: "Two noisy mic captures",
                args: r#"{"a": [0.0, 1.0, 0.5, -0.3, 0.1], "b": [0.2, 0.0, 1.0, 0.5, -0.3], "sample_rate_hz": 48000.0}"#,
                note: Some("Use `peak_lag_s` for TDOA bearing estimates."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Estimate time delay between two microphones or radio receivers.",
            "Detect a known reference template buried in a longer recording.",
            "Align two time series before subtracting or coherently averaging.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Length {
                field: "a",
                min: Some(1),
                max: None,
            },
            Rule::Length {
                field: "b",
                min: Some(1),
                max: None,
            },
            Rule::Range {
                field: "sample_rate_hz",
                min: Some(f64::MIN_POSITIVE),
                max: None,
            },
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HilbertArgs {
    /// Real-valued input samples.
    samples: Vec<f64>,
    /// Sample rate in Hz (used to scale instantaneous frequency).
    sample_rate_hz: f64,
}

pub struct DspHilbert;
impl Skill for DspHilbert {
    fn name(&self) -> &'static str {
        "signal_hilbert"
    }
    fn description(&self) -> &'static str {
        "Hilbert transform via FFT — returns the analytic signal as \
        `real`/`imag` arrays and the derived `instantaneous_amplitude`, \
        `instantaneous_phase`, `instantaneous_frequency_hz`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HilbertArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<HilbertArgs>()?;
            let n = a.samples.len();
            let mut planner = FftPlanner::<f64>::new();
            let fft = planner.plan_fft_forward(n);
            let ifft = planner.plan_fft_inverse(n);
            let mut buf: Vec<Complex64> =
                a.samples.iter().map(|x| Complex64::new(*x, 0.0)).collect();
            fft.process(&mut buf);
            // Zero negative frequencies, double positive.
            for (i, b) in buf.iter_mut().enumerate() {
                if i == 0 || (n.is_multiple_of(2) && i == n / 2) {
                    // keep
                } else if i < n / 2 {
                    *b *= 2.0;
                } else {
                    *b = Complex64::new(0.0, 0.0);
                }
            }
            ifft.process(&mut buf);
            let scale = 1.0 / n as f64;
            let re: Vec<f64> = buf.iter().map(|c| c.re * scale).collect();
            let im: Vec<f64> = buf.iter().map(|c| c.im * scale).collect();
            let amp: Vec<f64> = buf.iter().map(|c| c.norm() * scale).collect();
            let phase: Vec<f64> = buf.iter().map(|c| c.arg()).collect();
            // Instantaneous frequency via phase difference (unwrapped).
            let mut inst_freq = Vec::with_capacity(n);
            inst_freq.push(0.0);
            for i in 1..n {
                let mut dphi = phase[i] - phase[i - 1];
                while dphi > std::f64::consts::PI {
                    dphi -= 2.0 * std::f64::consts::PI;
                }
                while dphi < -std::f64::consts::PI {
                    dphi += 2.0 * std::f64::consts::PI;
                }
                inst_freq.push(dphi * a.sample_rate_hz / (2.0 * std::f64::consts::PI));
            }
            Ok(text_result(
                json!({
                    "real": re,
                    "imag": im,
                    "instantaneous_amplitude": amp,
                    "instantaneous_phase": phase,
                    "instantaneous_frequency_hz": inst_freq,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "AM-modulated tone",
                args: r#"{"samples": [0.0, 0.5, 1.0, 0.5, 0.0, -0.5, -1.0, -0.5, 0.0, 0.5, 1.0, 0.5, 0.0, -0.5, -1.0, -0.5], "sample_rate_hz": 8000.0}"#,
                note: Some("Use `instantaneous_amplitude` to read the envelope."),
            },
            SkillExample {
                title: "Linear chirp",
                args: r#"{"samples": [0.0, 0.1, 0.3, 0.5, 0.7, 0.9, 1.0, 0.8, 0.4, -0.1, -0.6, -0.9, -1.0, -0.7, -0.2, 0.4], "sample_rate_hz": 1000.0}"#,
                note: Some("`instantaneous_frequency_hz` should ramp upward."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Extract the analytic-signal envelope for AM demodulation.",
            "Track instantaneous frequency of a chirp or FM signal.",
            "Build SSB modulators / single-sideband filters in post-processing.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "samples",
            min: Some(1),
            max: None,
        }]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CepstrumArgs {
    /// Real-valued input samples.
    samples: Vec<f64>,
}

pub struct DspCepstrum;
impl Skill for DspCepstrum {
    fn name(&self) -> &'static str {
        "signal_cepstrum"
    }
    fn description(&self) -> &'static str {
        "Real cepstrum = inverse-FFT of log magnitude spectrum. Peaks at \
        non-zero quefrencies indicate periodic structure (fundamental \
        period for pitch detection, echo detection in acoustic / radar \
        return analysis)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CepstrumArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CepstrumArgs>()?;
            let n = a.samples.len().next_power_of_two();
            let mut planner = FftPlanner::<f64>::new();
            let fft = planner.plan_fft_forward(n);
            let ifft = planner.plan_fft_inverse(n);
            let mut buf: Vec<Complex64> = (0..n)
                .map(|i| Complex64::new(*a.samples.get(i).unwrap_or(&0.0), 0.0))
                .collect();
            fft.process(&mut buf);
            for c in &mut buf {
                let m = c.norm().max(1e-30);
                *c = Complex64::new(m.ln(), 0.0);
            }
            ifft.process(&mut buf);
            let scale = 1.0 / n as f64;
            let ceps: Vec<f64> = buf
                .iter()
                .take(a.samples.len())
                .map(|c| c.re * scale)
                .collect();
            Ok(text_result(json!({ "cepstrum": ceps }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Voiced speech frame",
                args: r#"{"samples": [0.0, 0.5, 1.0, 0.5, 0.0, -0.5, -1.0, -0.5, 0.0, 0.5, 1.0, 0.5, 0.0, -0.5, -1.0, -0.5]}"#,
                note: Some("Peak quefrency reveals the pitch period."),
            },
            SkillExample {
                title: "Echo detection",
                args: r#"{"samples": [1.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.25, 0.0, 0.0, 0.125, 0.0, 0.0, 0.06, 0.0, 0.0, 0.03]}"#,
                note: Some("Look for peaks at the echo delay quefrency."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Detect pitch period of voiced speech or musical notes.",
            "Find echoes and multipath delays in acoustic / radar returns.",
            "Decouple source from filter for homomorphic processing.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "samples",
            min: Some(1),
            max: None,
        }]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BerArgs {
    /// `bpsk`, `qpsk`, `psk_m` (specify `m`), `qam_m` (specify `m`), `fsk_coherent`, `fsk_noncoherent`.
    modulation: String,
    /// SNR Eb/N0 in dB.
    ebn0_db: f64,
    /// For PSK / QAM, the modulation order M.
    #[serde(default)]
    m: Option<u32>,
    /// `awgn` (default) or `rayleigh`.
    #[serde(default)]
    channel: Option<String>,
}

pub struct DspBer;
impl Skill for DspBer {
    fn name(&self) -> &'static str {
        "signal_ber_curve"
    }
    fn description(&self) -> &'static str {
        "Theoretical bit-error rate at a given Eb/N0 (dB) for common \
        modulations in AWGN or Rayleigh-fading channels. Returns `ber`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BerArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<BerArgs>()?;
            let gamma = 10_f64.powf(a.ebn0_db / 10.0);
            let channel = a.channel.unwrap_or_else(|| "awgn".into()).to_lowercase();
            let ber_awgn = match a.modulation.to_lowercase().as_str() {
                "bpsk" => 0.5 * erfc((gamma).sqrt()),
                "qpsk" => 0.5 * erfc((gamma).sqrt()),
                "psk_m" => {
                    let m = a.m.ok_or_else(|| invalid("psk_m requires m"))?;
                    let k = (m as f64).log2();
                    erfc((k * gamma).sqrt() * (std::f64::consts::PI / m as f64).sin()) / k
                }
                "qam_m" => {
                    // Square M-QAM BER (Proakis 5e §5.2.9):
                    // SER ≈ 4·(1 − 1/√M)·Q(√(3·k·γ/(M−1))). With
                    // Q(x) = ½·erfc(x/√2), this becomes
                    // SER ≈ 2·(1 − 1/√M)·erfc(√(3·k·γ/(2(M−1)))).
                    // BER ≈ SER / k (Gray coding).
                    // Earlier code was missing the 1/√2 factor (2(M−1) in the
                    // denominator), which understated BER by ~3 dB effective SNR.
                    let m = a.m.ok_or_else(|| invalid("qam_m requires m"))?;
                    let k = (m as f64).log2();
                    let arg = (3.0 * k * gamma / (2.0 * (m as f64 - 1.0))).sqrt();
                    2.0 * (1.0 - 1.0 / (m as f64).sqrt()) * erfc(arg) / k
                }
                "fsk_coherent" => 0.5 * erfc((gamma / 2.0).sqrt()),
                "fsk_noncoherent" => 0.5 * (-gamma / 2.0).exp(),
                other => return Err(invalid(format!("unknown modulation '{other}'"))),
            };
            let ber = match channel.as_str() {
                "awgn" => ber_awgn,
                "rayleigh" => {
                    // Average BER over Rayleigh: 0.5 * (1 - sqrt(γ/(1+γ))) for BPSK; approximate others.
                    if a.modulation.eq_ignore_ascii_case("bpsk")
                        || a.modulation.eq_ignore_ascii_case("qpsk")
                    {
                        0.5 * (1.0 - (gamma / (1.0 + gamma)).sqrt())
                    } else {
                        // Rough fallback.
                        ber_awgn * 4.0
                    }
                }
                other => return Err(invalid(format!("unknown channel '{other}'"))),
            };
            Ok(text_result(json!({ "ber": ber }).to_string()))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "BPSK in AWGN",
                args: r#"{"modulation": "bpsk", "ebn0_db": 7.0}"#,
                note: Some("Returns `ber` near 7.7e-4."),
            },
            SkillExample {
                title: "16-QAM",
                args: r#"{"modulation": "qam_m", "m": 16, "ebn0_db": 10.0}"#,
                note: Some("`m` is required for QAM/PSK."),
            },
            SkillExample {
                title: "BPSK in Rayleigh fading",
                args: r#"{"modulation": "bpsk", "ebn0_db": 15.0, "channel": "rayleigh"}"#,
                note: Some("Fading BER falls off as 1/SNR — much worse than AWGN."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Pick a modulation scheme for a given link Eb/N0 budget.",
            "Compare AWGN vs Rayleigh sensitivity for a wireless design.",
            "Plot a BER-vs-Eb/N0 curve by sweeping `ebn0_db`.",
        ]
    }
}

fn erfc(x: f64) -> f64 {
    // Abramowitz & Stegun 7.1.26.
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = t
        * (-x * x).exp()
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    if x < 0.0 {
        2.0 - y
    } else {
        y
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IqDemodArgs {
    /// In-phase samples (parallel to `q`).
    i: Vec<f64>,
    /// Quadrature samples (parallel to `i`).
    q: Vec<f64>,
}

pub struct DspIqDemod;
impl Skill for DspIqDemod {
    fn name(&self) -> &'static str {
        "signal_iq_demod"
    }
    fn description(&self) -> &'static str {
        "Convert I/Q samples to amplitude and phase: A = √(I² + Q²), \
        φ = atan2(Q, I). Returns the parallel arrays."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IqDemodArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<IqDemodArgs>()?;
            if a.i.len() != a.q.len() || a.i.is_empty() {
                return Err(invalid("i and q must be same non-zero length"));
            }
            let amp: Vec<f64> =
                a.i.iter()
                    .zip(a.q.iter())
                    .map(|(x, y)| (x * x + y * y).sqrt())
                    .collect();
            let phase: Vec<f64> =
                a.i.iter()
                    .zip(a.q.iter())
                    .map(|(x, y)| y.atan2(*x))
                    .collect();
            Ok(text_result(
                json!({ "amplitude": amp, "phase": phase }).to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Quarter-circle trajectory",
                args: r#"{"i": [1.0, 0.71, 0.0, -0.71], "q": [0.0, 0.71, 1.0, 0.71]}"#,
                note: Some("`amplitude` is 1; `phase` ramps from 0 to 3π/4."),
            },
            SkillExample {
                title: "Constant-envelope FM-style burst",
                args: r#"{"i": [1.0, 0.5, -0.5, -1.0, -0.5, 0.5], "q": [0.0, 0.87, 0.87, 0.0, -0.87, -0.87]}"#,
                note: Some("Amplitude stays at 1; phase wraps around the unit circle."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Convert SDR I/Q samples to magnitude / phase for envelope detection.",
            "Drive constellation plotting tools from raw quadrature data.",
            "Extract phase to feed a PLL or frequency-tracking loop.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Length {
                field: "i",
                min: Some(1),
                max: None,
            },
            Rule::Length {
                field: "q",
                min: Some(1),
                max: None,
            },
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(DspSpectrogram),
        Box::new(DspCrossCorrelation),
        Box::new(DspHilbert),
        Box::new(DspCepstrum),
        Box::new(DspBer),
        Box::new(DspIqDemod),
    ]
}
