//! SDR skill — list software-defined radios and sweep the spectrum by shelling
//! out to the standard CLI tools (`rtl_test`/`hackrf_info` for discovery,
//! `rtl_power` for a power sweep). **Off by default** (`[sdr].enabled`); requires
//! the relevant tools + hardware. **Receive-only** — there is deliberately no
//! transmit path. If the tools aren't installed, the error says so.

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use tokio::process::Command;

use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

/// Tool names (gated by `[sdr].enabled` in `disabled_by_config`).
pub const TOOL_NAMES: &[&str] = &["sdr_devices", "sdr_scan"];

/// Run a command with a deadline, killing it on timeout (kill-on-drop). Returns
/// stdout (lossy UTF-8) on success. Missing binary → a clear install hint.
async fn run(program: &str, args: &[String], timeout_secs: u64) -> Result<String> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow!("'{program}' not found on PATH — install the RTL-SDR / HackRF tools")
            } else {
                anyhow!("could not start '{program}': {e}")
            }
        })?;
    let out =
        match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output())
            .await
        {
            Ok(r) => r.map_err(|e| anyhow!("'{program}' failed: {e}"))?,
            Err(_) => return Err(anyhow!("'{program}' timed out after {timeout_secs}s")),
        };
    // rtl_test/hackrf_info write useful info to stderr too; include both.
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr);
    if !err.trim().is_empty() {
        text.push_str(&err);
    }
    Ok(text)
}

pub struct SdrDevices;
impl Skill for SdrDevices {
    fn name(&self) -> &'static str {
        "sdr_devices"
    }
    fn description(&self) -> &'static str {
        "List connected software-defined radios (off by default; [sdr]). Probes RTL-SDR via \
        `rtl_test` and HackRF via `hackrf_info`. Read-only; requires the tools + hardware."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let mut sections: Vec<String> = Vec::new();
            // RTL-SDR: rtl_test -t runs a quick test and prints the device list; bound it.
            match run("rtl_test", &["-t".to_string()], 5).await {
                Ok(o) => {
                    let header: String = o
                        .lines()
                        .take_while(|l| !l.contains("Sampling at") && !l.contains("Benchmarking"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    sections.push(format!("RTL-SDR (rtl_test):\n{}", header.trim()));
                }
                Err(e) => sections.push(format!("RTL-SDR: {e}")),
            }
            // HackRF: hackrf_info prints serial/board info and exits.
            match run("hackrf_info", &[], 5).await {
                Ok(o) if !o.trim().is_empty() => {
                    sections.push(format!("HackRF (hackrf_info):\n{}", o.trim()))
                }
                Ok(_) => {}
                Err(e) => sections.push(format!("HackRF: {e}")),
            }
            Ok(text_result(sections.join("\n\n")))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ScanArgs {
    /// Start frequency in MHz (e.g. 88 for the FM band).
    start_mhz: f64,
    /// End frequency in MHz (must be greater than `start_mhz`).
    end_mhz: f64,
    /// Frequency bin width in kHz (resolution; default 100).
    #[serde(default)]
    bin_khz: Option<f64>,
    /// How many of the strongest bins to report (default 15, capped at 100).
    #[serde(default)]
    top: Option<usize>,
}

pub struct SdrScan;
impl Skill for SdrScan {
    fn name(&self) -> &'static str {
        "sdr_scan"
    }
    fn description(&self) -> &'static str {
        "Sweep the RF spectrum and report the strongest signals (off by default; [sdr]). Runs a \
        single `rtl_power` sweep over start_mhz..end_mhz at bin_khz resolution and returns the \
        loudest bins (frequency + power in dB). Receive-only; requires an RTL-SDR + `rtl_power`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ScanArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ScanArgs>()?;
            if !(args.start_mhz.is_finite() && args.end_mhz.is_finite())
                || args.end_mhz <= args.start_mhz
            {
                return Err(invalid("require finite start_mhz < end_mhz"));
            }
            if args.start_mhz < 0.0 || args.end_mhz > 6000.0 {
                return Err(invalid("frequencies must be within 0–6000 MHz"));
            }
            let bin_khz = args.bin_khz.unwrap_or(100.0);
            if !(0.1..=10_000.0).contains(&bin_khz) {
                return Err(invalid("bin_khz must be between 0.1 and 10000"));
            }
            let top = args.top.unwrap_or(15).clamp(1, 100);

            let start_hz = (args.start_mhz * 1e6).round() as i64;
            let end_hz = (args.end_mhz * 1e6).round() as i64;
            let bin_hz = (bin_khz * 1e3).round() as i64;
            let freq_arg = format!("{start_hz}:{end_hz}:{bin_hz}");
            // `-1` = single sweep then exit; `-` = write CSV to stdout.
            let csv = run(
                "rtl_power",
                &["-f".into(), freq_arg, "-1".into(), "-".into()],
                90,
            )
            .await
            .map_err(internal)?;

            let bins = parse_rtl_power(&csv);
            if bins.is_empty() {
                return Err(invalid(
                    "rtl_power returned no data (no device, or range produced no bins)",
                ));
            }
            Ok(text_result(format_peaks(&bins, top, bin_hz)))
        })
    }
}

/// One scanned bin: center frequency (Hz) and power (dB).
struct Bin {
    freq_hz: f64,
    db: f64,
}

/// Parse `rtl_power` CSV. Each line: date, time, freq_low, freq_high, bin_step,
/// n_samples, then one dB value per bin. We reconstruct each bin's center freq.
fn parse_rtl_power(csv: &str) -> Vec<Bin> {
    let mut out = Vec::new();
    for line in csv.lines() {
        let f: Vec<&str> = line.split(',').map(str::trim).collect();
        if f.len() < 7 {
            continue;
        }
        let (Ok(freq_low), Ok(bin_step)) = (f[2].parse::<f64>(), f[4].parse::<f64>()) else {
            continue;
        };
        for (i, raw) in f[6..].iter().enumerate() {
            if let Ok(db) = raw.parse::<f64>() {
                if db.is_finite() {
                    out.push(Bin {
                        freq_hz: freq_low + bin_step * (i as f64 + 0.5),
                        db,
                    });
                }
            }
        }
    }
    out
}

/// Format a human-readable frequency (Hz → kHz/MHz/GHz).
fn fmt_freq(hz: f64) -> String {
    if hz >= 1e9 {
        format!("{:.4} GHz", hz / 1e9)
    } else if hz >= 1e6 {
        format!("{:.4} MHz", hz / 1e6)
    } else {
        format!("{:.2} kHz", hz / 1e3)
    }
}

/// Summarize a sweep: total bins, the floor/peak, and the strongest `top` bins.
fn format_peaks(bins: &[Bin], top: usize, bin_hz: i64) -> String {
    let mut idx: Vec<usize> = (0..bins.len()).collect();
    idx.sort_by(|&a, &b| {
        bins[b]
            .db
            .partial_cmp(&bins[a].db)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let min = bins.iter().map(|b| b.db).fold(f64::INFINITY, f64::min);
    let max = bins.iter().map(|b| b.db).fold(f64::NEG_INFINITY, f64::max);
    let mut lines = vec![format!(
        "Swept {} bins @ {} kHz; floor {:.1} dB, peak {:.1} dB. Strongest {}:",
        bins.len(),
        bin_hz as f64 / 1e3,
        min,
        max,
        top.min(bins.len()),
    )];
    for &i in idx.iter().take(top) {
        lines.push(format!(
            "  {}  {:.1} dB",
            fmt_freq(bins[i].freq_hz),
            bins[i].db
        ));
    }
    lines.join("\n")
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(SdrDevices), Box::new(SdrScan)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rtl_power_csv() {
        // freq_low=88M, freq_high=88.3M, bin_step=100k, 3 bins → -40,-30,-50 dB.
        let csv = "2026-01-01, 12:00:00, 88000000, 88300000, 100000, 1024, -40.0, -30.0, -50.0\n";
        let bins = parse_rtl_power(csv);
        assert_eq!(bins.len(), 3);
        // First bin center = 88M + 100k*0.5 = 88.05M.
        assert!((bins[0].freq_hz - 88_050_000.0).abs() < 1.0);
        assert_eq!(bins[1].db, -30.0);
    }

    #[test]
    fn peaks_sorted_strongest_first() {
        let csv = "d, t, 100000000, 100300000, 100000, 1, -80.0, -20.0, -55.0\n";
        let bins = parse_rtl_power(csv);
        let out = format_peaks(&bins, 2, 100000);
        // The -20 dB bin (2nd, center 100.15 MHz) is strongest → listed first.
        let first_peak_line = out.lines().nth(1).unwrap();
        assert!(first_peak_line.contains("-20.0 dB"), "{out}");
        assert!(first_peak_line.contains("100.1500 MHz"), "{out}");
    }

    #[test]
    fn freq_units() {
        assert_eq!(fmt_freq(433_920_000.0), "433.9200 MHz");
        assert_eq!(fmt_freq(2_400_000_000.0), "2.4000 GHz");
        assert_eq!(fmt_freq(125_000.0), "125.00 kHz");
    }

    #[test]
    fn ignores_malformed_lines() {
        let csv = "garbage\n\nd,t,1000,2000\n";
        assert!(parse_rtl_power(csv).is_empty());
    }
}
