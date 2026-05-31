//! FFmpeg skill — probe and convert local media by shelling out to the system
//! `ffmpeg`/`ffprobe`. **Off by default** (`[ffmpeg].enabled`). Every input/output
//! path is confined to `[filesystem].roots` (same rules as the filesystem skill),
//! and `ffmpeg_convert` (which writes a file) goes through the confirmation
//! [`guard`](crate::skills::guard). If the binary isn't on `PATH`, the error says so.

use std::process::Stdio;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;
use tokio::process::Command;

use crate::skills::filesystem::resolve;
use crate::skills::guard::Decision;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::human_size;
use crate::{internal, invalid, text_result};

/// Tool names (gated by `[ffmpeg].enabled` in `disabled_by_config`).
pub const TOOL_NAMES: &[&str] = &["ffmpeg_probe", "ffmpeg_convert"];

/// Map a spawn failure to a clear message (missing binary → install hint).
fn spawn_err(program: &str, e: std::io::Error) -> anyhow::Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        anyhow!("'{program}' not found on PATH — install FFmpeg to use this tool")
    } else {
        anyhow!("could not start '{program}': {e}")
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ProbeArgs {
    /// Path to the media file (confined to [filesystem].roots).
    input: String,
}

pub struct FfmpegProbe;
impl Skill for FfmpegProbe {
    fn name(&self) -> &'static str {
        "ffmpeg_probe"
    }
    fn description(&self) -> &'static str {
        "Inspect a local media file with ffprobe (read-only): container format, duration, bitrate, \
        and per-stream codec/resolution/sample-rate. Path is confined to [filesystem].roots. \
        Requires a local FFmpeg install ([ffmpeg], off by default)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ProbeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ProbeArgs>()?;
            let input = resolve(&server.fs, &args.input)?;
            let out = Command::new("ffprobe")
                .args([
                    "-v",
                    "quiet",
                    "-print_format",
                    "json",
                    "-show_format",
                    "-show_streams",
                ])
                .arg(&input)
                .output()
                .await
                .map_err(|e| internal(spawn_err("ffprobe", e)))?;
            if !out.status.success() {
                return Err(invalid(format!(
                    "ffprobe failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                )));
            }
            let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| internal(e.into()))?;
            Ok(text_result(summarize_probe(&v, &args.input)))
        })
    }
}

/// Render a concise human summary from ffprobe's JSON.
fn summarize_probe(v: &Value, label: &str) -> String {
    let mut lines = vec![format!("{label}")];
    if let Some(fmt) = v.get("format") {
        let name = fmt
            .get("format_long_name")
            .or_else(|| fmt.get("format_name"))
            .and_then(Value::as_str)
            .unwrap_or("?");
        let dur = fmt
            .get("duration")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<f64>().ok());
        let br = fmt
            .get("bit_rate")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<f64>().ok());
        let mut l = format!("  format: {name}");
        if let Some(d) = dur {
            l.push_str(&format!("  duration: {}", fmt_duration(d)));
        }
        if let Some(b) = br {
            l.push_str(&format!("  bitrate: {} kb/s", (b / 1000.0).round() as i64));
        }
        lines.push(l);
    }
    if let Some(streams) = v.get("streams").and_then(Value::as_array) {
        for s in streams {
            let idx = s.get("index").and_then(Value::as_i64).unwrap_or(-1);
            let kind = s.get("codec_type").and_then(Value::as_str).unwrap_or("?");
            let codec = s.get("codec_name").and_then(Value::as_str).unwrap_or("?");
            let mut detail = String::new();
            match kind {
                "video" => {
                    let w = s.get("width").and_then(Value::as_i64).unwrap_or(0);
                    let h = s.get("height").and_then(Value::as_i64).unwrap_or(0);
                    if w > 0 && h > 0 {
                        detail = format!(" {w}x{h}");
                    }
                    if let Some(fr) = s.get("r_frame_rate").and_then(Value::as_str) {
                        detail.push_str(&format!(" @ {fr} fps"));
                    }
                }
                "audio" => {
                    if let Some(sr) = s.get("sample_rate").and_then(Value::as_str) {
                        detail.push_str(&format!(" {sr} Hz"));
                    }
                    if let Some(ch) = s.get("channels").and_then(Value::as_i64) {
                        detail.push_str(&format!(" {ch}ch"));
                    }
                }
                _ => {}
            }
            lines.push(format!("  stream #{idx} {kind}: {codec}{detail}"));
        }
    }
    lines.join("\n")
}

fn fmt_duration(secs: f64) -> String {
    let total = secs.round() as i64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ConvertArgs {
    /// Source media path (confined to [filesystem].roots).
    input: String,
    /// Destination path (confined to [filesystem].roots). The container/format is
    /// inferred from its extension unless overridden in `args`.
    output: String,
    /// Extra ffmpeg arguments, pre-split (no shell), inserted between `-i input` and
    /// `output` — e.g. `["-vf", "scale=1280:-1", "-c:v", "libx264", "-crf", "23"]`.
    #[serde(default)]
    args: Option<Vec<String>>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, stop asking for ffmpeg_convert for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

pub struct FfmpegConvert;
impl Skill for FfmpegConvert {
    fn name(&self) -> &'static str {
        "ffmpeg_convert"
    }
    fn description(&self) -> &'static str {
        "Convert/transcode a local media file with ffmpeg (off by default; [ffmpeg]). Writes a \
        file, so the first call returns a confirmation token and does nothing; call again with \
        confirm=<token> to run (or confirm + trust=true). Both paths are confined to \
        [filesystem].roots; pass extra ffmpeg flags via `args` (pre-split). Large conversions may \
        exceed the client's tool timeout."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ConvertArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ConvertArgs>()?;
            let input = resolve(&server.fs, &args.input)?;
            let output = resolve(&server.fs, &args.output)?;
            let extra = args.args.unwrap_or_default();

            let summary = format!(
                "ffmpeg convert {} -> {}{}",
                args.input,
                args.output,
                if extra.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", extra.join(" "))
                }
            );
            if let Decision::Challenge(msg) = server.guard.check(
                "ffmpeg_convert",
                "ffmpeg_convert",
                false,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }

            let mut cmd = Command::new("ffmpeg");
            cmd.arg("-hide_banner")
                .arg("-y")
                .arg("-i")
                .arg(&input)
                .args(&extra)
                .arg(&output)
                .stdin(Stdio::null());
            let out = cmd
                .output()
                .await
                .map_err(|e| internal(spawn_err("ffmpeg", e)))?;
            if !out.status.success() {
                let err = String::from_utf8_lossy(&out.stderr);
                let tail: String = err
                    .lines()
                    .rev()
                    .take(8)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n");
                return Err(invalid(format!("ffmpeg failed:\n{tail}")));
            }
            let size = tokio::fs::metadata(&output)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            Ok(text_result(format!(
                "Wrote {} ({}).",
                args.output,
                human_size(size)
            )))
        })
    }
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(FfmpegProbe), Box::new(FfmpegConvert)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_formats() {
        assert_eq!(fmt_duration(65.0), "1:05");
        assert_eq!(fmt_duration(3725.0), "1:02:05");
    }

    #[test]
    fn bytes_format() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn summarize_handles_video_and_audio() {
        let v: Value = serde_json::from_str(
            r#"{"format":{"format_name":"mov,mp4","duration":"12.5","bit_rate":"800000"},
                "streams":[
                  {"index":0,"codec_type":"video","codec_name":"h264","width":1920,"height":1080},
                  {"index":1,"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":2}
                ]}"#,
        )
        .unwrap();
        let s = summarize_probe(&v, "clip.mp4");
        assert!(s.contains("clip.mp4"));
        assert!(s.contains("1920x1080"));
        assert!(s.contains("h264"));
        assert!(s.contains("48000 Hz"));
        assert!(s.contains("2ch"));
    }
}
