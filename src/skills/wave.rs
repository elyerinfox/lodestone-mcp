//! WAV file skills — probe and decode `.wav` files via `hound`. Off by default
//! (`[wave].enabled`). Paths are confined to `[filesystem].roots`.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::filesystem;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

pub const TOOL_NAMES: &[&str] = &["wave_info", "wave_samples"];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PathArgs {
    /// Path to a WAV file (must be inside one of `[filesystem].roots`).
    path: String,
}

pub struct WaveInfo;
impl Skill for WaveInfo {
    fn name(&self) -> &'static str {
        "wave_info"
    }
    fn description(&self) -> &'static str {
        "Probe a WAV file: sample rate, channel count, bit depth, sample format (int/float), \
        total samples, and duration. Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PathArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PathArgs>()?;
            let p = filesystem::resolve(&server.fs, &args.path)?;
            let reader = hound::WavReader::open(&p)
                .map_err(|e| internal(anyhow::anyhow!("hound open {}: {e}", p.display())))?;
            let spec = reader.spec();
            let total = reader.duration() as u64;
            let dur = total as f64 / spec.sample_rate as f64;
            Ok(text_result(format!(
                "WAV {}\n  sample rate: {} Hz\n  channels: {}\n  bits/sample: {}\n  format: {:?}\n  total frames: {}\n  duration: {:.3}s",
                p.display(),
                spec.sample_rate,
                spec.channels,
                spec.bits_per_sample,
                spec.sample_format,
                total,
                dur,
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SamplesArgs {
    /// Path to a WAV file.
    path: String,
    /// Max samples to return (default 1024, capped at 100000). Reading the entire
    /// file is rarely useful in a tool call — pair with signal_fft on a slice.
    #[serde(default)]
    max_samples: Option<u32>,
    /// Which channel to extract from a multi-channel file (default 0).
    #[serde(default)]
    channel: Option<u32>,
}

pub struct WaveSamples;
impl Skill for WaveSamples {
    fn name(&self) -> &'static str {
        "wave_samples"
    }
    fn description(&self) -> &'static str {
        "Decode samples from a WAV file as floats in [-1, 1]. Returns one channel \
        (`channel`, default 0) up to `max_samples`. Feed the array into signal_fft / \
        signal_dominant_frequencies / signal_rms."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SamplesArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SamplesArgs>()?;
            let p = filesystem::resolve(&server.fs, &args.path)?;
            let max = args.max_samples.unwrap_or(1024).clamp(1, 100_000) as usize;
            let channel = args.channel.unwrap_or(0) as usize;
            let mut reader = hound::WavReader::open(&p)
                .map_err(|e| internal(anyhow::anyhow!("hound open {}: {e}", p.display())))?;
            let spec = reader.spec();
            if channel >= spec.channels as usize {
                return Err(invalid(format!(
                    "channel {channel} out of range (file has {} channel(s))",
                    spec.channels
                )));
            }
            let stride = spec.channels as usize;
            let max_frames = max;
            let samples: Vec<f64> = match spec.sample_format {
                hound::SampleFormat::Int => {
                    let scale = 1.0 / (1i64 << (spec.bits_per_sample - 1)) as f64;
                    let mut frames = reader.samples::<i32>();
                    let mut out = Vec::with_capacity(max_frames);
                    let mut buf = vec![0i32; stride];
                    while out.len() < max_frames {
                        for slot in buf.iter_mut() {
                            match frames.next() {
                                Some(Ok(v)) => *slot = v,
                                Some(Err(e)) => {
                                    return Err(internal(anyhow::anyhow!("decode error: {e}")))
                                }
                                None => return Ok(make_result(&p, channel, &out, max)),
                            }
                        }
                        out.push(buf[channel] as f64 * scale);
                    }
                    out
                }
                hound::SampleFormat::Float => {
                    let mut frames = reader.samples::<f32>();
                    let mut out = Vec::with_capacity(max_frames);
                    let mut buf = vec![0f32; stride];
                    while out.len() < max_frames {
                        for slot in buf.iter_mut() {
                            match frames.next() {
                                Some(Ok(v)) => *slot = v,
                                Some(Err(e)) => {
                                    return Err(internal(anyhow::anyhow!("decode error: {e}")))
                                }
                                None => return Ok(make_result(&p, channel, &out, max)),
                            }
                        }
                        out.push(buf[channel] as f64);
                    }
                    out
                }
            };
            Ok(make_result(&p, channel, &samples, max))
        })
    }
}

fn make_result(p: &std::path::Path, channel: usize, samples: &[f64], cap: usize) -> CallToolResult {
    let preview: Vec<String> = samples
        .iter()
        .take(16)
        .map(|v| format!("{v:.4}"))
        .collect();
    text_result(format!(
        "WAV {} — channel {}, {} samples (cap {})\nFirst {}: [{}]{}",
        p.display(),
        channel,
        samples.len(),
        cap,
        preview.len(),
        preview.join(", "),
        if samples.len() > preview.len() {
            ", …"
        } else {
            ""
        }
    ))
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(WaveInfo), Box::new(WaveSamples)]
}
