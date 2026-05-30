//! System-information skills — read-only host facts: OS, CPU, memory, disks, and
//! (where present) NVIDIA GPU stats. A local-system capability alongside
//! docker/k8s/fs; gated by `[sysinfo]` (on by default — purely read-only).
//!
//! Cross-platform via the `sysinfo` crate (Linux reads `/proc`/`/sys`, Windows uses
//! the OS APIs). GPU stats use NVML (`nvml-wrapper`), loaded at runtime — when the
//! NVML library/driver is absent the GPU tool returns a clear message rather than
//! failing the server (dependency safeguard, golden rule).

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;

use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::util::human_size;
use crate::{internal, text_result};

/// Tool names for this family (gated by `[sysinfo].enabled` in `disabled_by_config`).
pub const TOOL_NAMES: &[&str] = &[
    "system_info",
    "system_disks",
    "system_gpu",
    "system_os_release",
];

/// Host/OS/CPU/memory summary. Blocking work (sysinfo refresh + a short CPU
/// sampling interval), so callers run it on a blocking thread.
fn gather_info() -> String {
    use ::sysinfo::System;

    let mut sys = System::new_all();
    // A second refresh after a brief interval makes CPU usage meaningful (it's a
    // delta between samples).
    std::thread::sleep(std::time::Duration::from_millis(250));
    sys.refresh_all();

    let os = System::long_os_version()
        .or_else(System::os_version)
        .unwrap_or_else(|| "unknown".into());
    let kernel = System::kernel_version().unwrap_or_else(|| "unknown".into());
    let host = System::host_name().unwrap_or_else(|| "unknown".into());
    let uptime = format_duration(System::uptime());

    let cpus = sys.cpus();
    let cpu_brand = cpus.first().map(|c| c.brand().trim().to_string());
    let cpu_brand = cpu_brand.filter(|b| !b.is_empty()).unwrap_or_else(|| {
        cpus.first()
            .map(|c| c.vendor_id().to_string())
            .unwrap_or_default()
    });
    let ghz = cpus.first().map(|c| c.frequency()).unwrap_or(0) as f64 / 1000.0;

    format!(
        "Host: {host}\nOS: {os}\nKernel: {kernel}\nUptime: {uptime}\n\
         CPU: {cpu_brand} ({} logical cores @ ~{ghz:.2} GHz)\n\
         CPU usage: {:.1}%\n\
         Memory: {} used / {} total\n\
         Swap: {} used / {} total",
        cpus.len(),
        sys.global_cpu_usage(),
        human_size(sys.used_memory()),
        human_size(sys.total_memory()),
        human_size(sys.used_swap()),
        human_size(sys.total_swap()),
    )
}

/// Mounted disks: mount point, filesystem, capacity, and free space.
fn gather_disks() -> String {
    use ::sysinfo::Disks;

    let disks = Disks::new_with_refreshed_list();
    let list = disks.list();
    if list.is_empty() {
        return "No disks reported.".into();
    }
    let mut out = format!("Disks ({}):\n", list.len());
    for d in list {
        let total = d.total_space();
        let avail = d.available_space();
        let used = total.saturating_sub(avail);
        let pct = if total > 0 {
            used as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        out.push_str(&format!(
            "\n  {} ({})\n    mount: {}\n    {} used / {} total ({:.0}% used), {} free\n",
            d.name().to_string_lossy(),
            d.file_system().to_string_lossy(),
            d.mount_point().display(),
            human_size(used),
            human_size(total),
            pct,
            human_size(avail),
        ));
    }
    out
}

/// NVIDIA GPU stats via NVML, or a clear message when NVML/driver isn't available.
fn gather_gpu() -> String {
    use ::nvml_wrapper::enum_wrappers::device::TemperatureSensor;
    use ::nvml_wrapper::Nvml;

    let nvml = match Nvml::init() {
        Ok(n) => n,
        Err(e) => {
            return format!(
                "No NVIDIA GPU stats available — NVML could not be loaded ({e}). This needs an \
                 NVIDIA driver with the NVML library (nvml.dll / libnvidia-ml.so). Non-NVIDIA GPUs \
                 aren't supported."
            )
        }
    };
    let count = match nvml.device_count() {
        Ok(c) => c,
        Err(e) => return format!("Could not query GPU count: {e}"),
    };
    if count == 0 {
        return "NVML loaded but reports 0 GPUs.".into();
    }
    let mut out = format!("NVIDIA GPUs ({count}):\n");
    for i in 0..count {
        let dev = match nvml.device_by_index(i) {
            Ok(d) => d,
            Err(e) => {
                out.push_str(&format!("\n  GPU {i}: error: {e}\n"));
                continue;
            }
        };
        let name = dev.name().unwrap_or_else(|_| "unknown".into());
        out.push_str(&format!("\n  GPU {i}: {name}\n"));
        if let Ok(mem) = dev.memory_info() {
            out.push_str(&format!(
                "    memory: {} used / {} total\n",
                human_size(mem.used),
                human_size(mem.total)
            ));
        }
        if let Ok(u) = dev.utilization_rates() {
            out.push_str(&format!(
                "    utilization: {}% gpu, {}% memory\n",
                u.gpu, u.memory
            ));
        }
        if let Ok(t) = dev.temperature(TemperatureSensor::Gpu) {
            out.push_str(&format!("    temperature: {t} °C\n"));
        }
    }
    out
}

/// Run blocking sysinfo/NVML work off the async runtime, mapping join failures.
async fn blocking<F>(f: F) -> Result<String, McpError>
where
    F: FnOnce() -> String + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| internal(anyhow::anyhow!("system-info task failed: {e}")))
}

pub struct SystemInfo;
impl Skill for SystemInfo {
    fn name(&self) -> &'static str {
        "system_info"
    }
    fn description(&self) -> &'static str {
        "Report this machine's host name, OS/kernel, uptime, CPU (model, cores, usage), and memory/\
        swap usage. Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(blocking(gather_info).await?)) })
    }
}

pub struct SystemDisks;
impl Skill for SystemDisks {
    fn name(&self) -> &'static str {
        "system_disks"
    }
    fn description(&self) -> &'static str {
        "List mounted disks/volumes on this machine with filesystem, total/used/free space. \
        Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(blocking(gather_disks).await?)) })
    }
}

pub struct SystemGpu;
impl Skill for SystemGpu {
    fn name(&self) -> &'static str {
        "system_gpu"
    }
    fn description(&self) -> &'static str {
        "Report NVIDIA GPU stats (name, memory, utilization, temperature) via NVML. Returns a clear \
        message if no NVIDIA GPU / NVML library is present. Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(blocking(gather_gpu).await?)) })
    }
}

/// `123s` → `2h 3m`-style compact duration.
fn format_duration(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3_600;
    let m = (secs % 3_600) / 60;
    let mut parts = Vec::new();
    if d > 0 {
        parts.push(format!("{d}d"));
    }
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 || parts.is_empty() {
        parts.push(format!("{m}m"));
    }
    parts.join(" ")
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
/// Parse the simple KEY=VALUE / KEY="VALUE" format used by `/etc/os-release`
/// (and `/usr/lib/os-release` as a fallback) per the systemd man page. On
/// non-Linux hosts the file is absent and we say so.
pub struct SystemOsRelease;
impl Skill for SystemOsRelease {
    fn name(&self) -> &'static str {
        "system_os_release"
    }
    fn description(&self) -> &'static str {
        "Read and parse `/etc/os-release` (Linux distro identifier per the systemd spec). \
        Returns NAME, VERSION, ID, ID_LIKE, PRETTY_NAME, VERSION_ID, HOME_URL, etc. On \
        non-Linux hosts (or when the file is missing) says so."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let candidates = ["/etc/os-release", "/usr/lib/os-release"];
            let mut found: Option<(String, String)> = None;
            for path in candidates {
                if let Ok(s) = tokio::fs::read_to_string(path).await {
                    found = Some((path.to_string(), s));
                    break;
                }
            }
            let Some((path, contents)) = found else {
                return Ok(text_result(
                    "os-release file not present (typical on non-Linux hosts).".to_string(),
                ));
            };
            let mut out = format!("Parsed from {path}:\n");
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    let v = v.trim();
                    let v = v.strip_prefix('"').unwrap_or(v);
                    let v = v.strip_suffix('"').unwrap_or(v);
                    out.push_str(&format!("  {k} = {v}\n"));
                }
            }
            Ok(text_result(out))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(SystemInfo),
        Box::new(SystemDisks),
        Box::new(SystemGpu),
        Box::new(SystemOsRelease),
    ]
}

#[cfg(test)]
mod tests {
    use super::format_duration;

    #[test]
    fn duration_formats_compactly() {
        assert_eq!(format_duration(0), "0m");
        assert_eq!(format_duration(90), "1m");
        assert_eq!(format_duration(3_600), "1h");
        assert_eq!(format_duration(90_061), "1d 1h 1m");
    }
}
