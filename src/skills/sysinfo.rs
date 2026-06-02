//! System-information skills — read-only host facts: OS, CPU, memory, disks, and
//! GPU stats across NVIDIA / AMD / Intel where the host exposes them. A
//! local-system capability alongside docker/k8s/fs; gated by `[sysinfo]` (on by
//! default — purely read-only).
//!
//! Cross-platform via the `sysinfo` crate (Linux reads `/proc`/`/sys`, Windows uses
//! the OS APIs). GPU stats are exposed as three distinct tools — one per vendor —
//! because the underlying backends are genuinely different methodologies (golden
//! rule 9: one tool per method):
//!   - `system_gpu_nvidia` — NVML via `nvml-wrapper`. Cross-platform; needs the
//!     NVIDIA driver with its NVML library (`nvml.dll` on Windows,
//!     `libnvidia-ml.so` on Linux).
//!   - `system_gpu_amd` — Linux DRM sysfs (`/sys/class/drm/card*/device/`) for
//!     `amdgpu`; reads VRAM totals, busy %, and hwmon temperatures.
//!   - `system_gpu_intel` — Linux DRM sysfs for `i915` / `xe`; reads frequency
//!     and hwmon temperatures.
//!
//! Each tool has its own per-tool capability gate so the LLM picks the one that
//! is actually available without guessing. On Windows / macOS, AMD and Intel
//! reads aren't currently surfaced (would need vendor-specific SDKs like ADL /
//! IGCL / IOKit).

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;

use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::util::human_size;
use crate::{internal, text_result};

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

/// PCI vendor IDs we match in the Linux DRM sysfs surface. NVIDIA goes
/// through NVML cross-platform so it doesn't need an ID here.
#[cfg(target_os = "linux")]
const VENDOR_AMD: u32 = 0x1002;
#[cfg(target_os = "linux")]
const VENDOR_INTEL: u32 = 0x8086;

/// Walk `/sys/class/drm/card[0-9]+/device/vendor` looking for `vendor_id`.
/// Returns true on first hit so a multi-card system doesn't hammer sysfs.
/// Subnodes (`card0-DP-1`, etc.) are skipped — they're connectors, not GPUs.
#[cfg(target_os = "linux")]
fn linux_drm_has_vendor(vendor_id: u32) -> bool {
    use std::fs;
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !(name_str.starts_with("card")
            && name_str.len() > 4
            && name_str[4..].bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let vendor_path = entry.path().join("device").join("vendor");
        if let Ok(content) = fs::read_to_string(&vendor_path) {
            if let Ok(n) = u32::from_str_radix(content.trim().trim_start_matches("0x"), 16) {
                if n == vendor_id {
                    return true;
                }
            }
        }
    }
    false
}

/// AMD wrapper: Linux uses the DRM sysfs path; on Windows / macOS we
/// have no surface here so we return a clear platform-mismatch
/// message. (Capability check should keep us from being called on
/// those, but the function still has to compile.)
fn gather_amd_gpu() -> String {
    #[cfg(target_os = "linux")]
    {
        gather_drm_sysfs_gpu("AMD", VENDOR_AMD)
    }
    #[cfg(not(target_os = "linux"))]
    {
        "AMD GPU stats are read via the Linux DRM sysfs (/sys/class/drm). \
         This host is not Linux; AMD GPUs on Windows / macOS would need a \
         vendor SDK (ADL / IOKit) that isn't integrated."
            .into()
    }
}

/// Intel wrapper: same Linux-only sysfs surface as AMD.
fn gather_intel_gpu() -> String {
    #[cfg(target_os = "linux")]
    {
        gather_drm_sysfs_gpu("Intel", VENDOR_INTEL)
    }
    #[cfg(not(target_os = "linux"))]
    {
        "Intel GPU stats are read via the Linux DRM sysfs (/sys/class/drm). \
         This host is not Linux; Intel GPUs on Windows / macOS would need a \
         vendor SDK (IGCL / IOKit) that isn't integrated."
            .into()
    }
}

fn gather_nvidia_gpu() -> String {
    use ::nvml_wrapper::enum_wrappers::device::TemperatureSensor;
    use ::nvml_wrapper::Nvml;

    let nvml = match Nvml::init() {
        Ok(n) => n,
        Err(e) => return format!("NVIDIA: NVML could not be loaded ({e})."),
    };
    let count = match nvml.device_count() {
        Ok(c) => c,
        Err(e) => return format!("NVIDIA: could not query GPU count: {e}"),
    };
    if count == 0 {
        return "NVIDIA: NVML loaded but reports 0 GPUs.".into();
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

/// Walks `/sys/class/drm/card[0-9]+/device/` for cards matching
/// `vendor_id` and reports busy %, VRAM, and hwmon temperature. Used by
/// both AMD (`amdgpu` driver fills mem_info_* + gpu_busy_percent) and
/// Intel (`i915`/`xe` drivers expose less detail but still hwmon temps
/// and a gt_act_freq_mhz that's a useful proxy).
#[cfg(target_os = "linux")]
fn gather_drm_sysfs_gpu(label: &str, vendor_id: u32) -> String {
    use std::fs;

    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return format!("{label}: /sys/class/drm not readable.");
    };
    let mut cards: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_string();
        if !(name_str.starts_with("card")
            && name_str.len() > 4
            && name_str[4..].bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let device_dir = entry.path().join("device");
        let vendor_ok = fs::read_to_string(device_dir.join("vendor"))
            .ok()
            .and_then(|s| u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok())
            .map(|n| n == vendor_id)
            .unwrap_or(false);
        if vendor_ok {
            cards.push((name_str, device_dir));
        }
    }
    if cards.is_empty() {
        return format!("{label}: no matching DRM cards.");
    }
    cards.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = format!("{label} GPUs ({}):\n", cards.len());
    for (card, dev_dir) in cards {
        let device_id = fs::read_to_string(dev_dir.join("device"))
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let model_hint = drm_model_hint(&dev_dir).unwrap_or_else(|| {
            if device_id.is_empty() {
                "unknown".into()
            } else {
                format!("device {device_id}")
            }
        });
        out.push_str(&format!("\n  {card}: {model_hint}\n"));
        // AMD's amdgpu publishes these; Intel's i915/xe doesn't fill
        // them all but we try every read uniformly and skip what's
        // empty / unsupported so the output stays clean either way.
        if let Some(busy) = read_u64(&dev_dir.join("gpu_busy_percent")) {
            out.push_str(&format!("    utilization: {busy}% gpu\n"));
        }
        if let Some(total) = read_u64(&dev_dir.join("mem_info_vram_total")) {
            let used = read_u64(&dev_dir.join("mem_info_vram_used")).unwrap_or(0);
            out.push_str(&format!(
                "    memory: {} used / {} total\n",
                human_size(used),
                human_size(total)
            ));
        }
        if let Some(freq_now) = read_u64(&dev_dir.join("gt_act_freq_mhz")) {
            let freq_max = read_u64(&dev_dir.join("gt_max_freq_mhz"));
            match freq_max {
                Some(max) if max > 0 => {
                    out.push_str(&format!("    frequency: {freq_now} / {max} MHz\n"))
                }
                _ => out.push_str(&format!("    frequency: {freq_now} MHz\n")),
            }
        }
        if let Some(temp_c) = read_hwmon_temp(&dev_dir) {
            out.push_str(&format!("    temperature: {temp_c} °C\n"));
        }
    }
    out
}

/// `/sys/class/drm/<card>/device/product_name` exists on some setups
/// (DG2/Arc on recent Intel, certain AMD configs). Falls back to None
/// when the file isn't there so `gather_drm_sysfs_gpu` can synthesize a
/// "device <pci-id>" label.
#[cfg(target_os = "linux")]
fn drm_model_hint(device_dir: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(device_dir.join("product_name"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Read the first `hwmon*/temp1_input` (millidegrees C) under the
/// device dir and return whole-degree C. Skips if hwmon isn't
/// populated (Intel without thermal driver, AMD on older kernels).
#[cfg(target_os = "linux")]
fn read_hwmon_temp(device_dir: &std::path::Path) -> Option<i64> {
    let hwmon_root = device_dir.join("hwmon");
    let entries = std::fs::read_dir(&hwmon_root).ok()?;
    for entry in entries.flatten() {
        let temp_path = entry.path().join("temp1_input");
        if let Some(milli) = read_u64(&temp_path) {
            return Some((milli / 1000) as i64);
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn read_u64(path: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[SkillExample {
            title: "Snapshot the host",
            args: r#"{}"#,
            note: Some("Returns host name, OS/kernel, uptime, CPU brand + usage, memory + swap."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Identify the host and OS the server is running on.",
            "Spot-check CPU and memory pressure before launching expensive work.",
            "Report the running machine in a status / sitrep response.",
        ]
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[SkillExample {
            title: "List mounted volumes",
            args: r#"{}"#,
            note: Some("Each entry shows mount point, filesystem, total/used/free space."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Check free space before downloading or generating a large artifact.",
            "Find the device/mount backing a path the user mentioned.",
            "Report disk-pressure as part of a host health check.",
        ]
    }
}

/// One tool per vendor (golden rule 9): each backend is a genuinely
/// different methodology — NVML is a userspace driver library;
/// AMD / Intel readings come from kernel-published DRM sysfs nodes —
/// and each has its own per-tool capability gate so the LLM picks
/// the one that's actually available without guessing.
pub struct SystemGpuNvidia;
impl Skill for SystemGpuNvidia {
    fn name(&self) -> &'static str {
        "system_gpu_nvidia"
    }
    fn description(&self) -> &'static str {
        "Report NVIDIA GPU stats (name, memory, utilization, temperature) via NVML. Cross-platform \
        — works wherever the NVIDIA driver ships its NVML library (`nvml.dll` / `libnvidia-ml.so`). \
        Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(blocking(gather_nvidia_gpu).await?)) })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[SkillExample {
            title: "Read NVIDIA GPU stats",
            args: r#"{}"#,
            note: Some("Requires the NVIDIA driver / NVML; errors clearly when missing."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Check NVIDIA GPU memory / utilization / temperature on this host.",
            "Confirm an NVIDIA GPU is visible to userspace before launching CUDA work.",
        ]
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        use crate::skills::SkillCapability;
        match nvml_wrapper::Nvml::init() {
            Ok(_) => SkillCapability::Ready,
            Err(e) => SkillCapability::unavailable(
                format!("NVML not loadable: {e}"),
                "install the NVIDIA driver (or `nvidia-utils` in containers) — \
                 this tool requires an NVIDIA GPU with NVML",
            ),
        }
    }
}

pub struct SystemGpuAmd;
impl Skill for SystemGpuAmd {
    fn name(&self) -> &'static str {
        "system_gpu_amd"
    }
    fn description(&self) -> &'static str {
        "Report AMD GPU stats (model, VRAM, busy %, hwmon temperature) by reading the Linux DRM \
        sysfs nodes the `amdgpu` kernel driver publishes under `/sys/class/drm/card*/device/`. \
        Linux-only. Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(blocking(gather_amd_gpu).await?)) })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[SkillExample {
            title: "Read AMD GPU stats (Linux)",
            args: r#"{}"#,
            note: Some("Reads `/sys/class/drm/card*/device/` from the amdgpu driver; Linux-only."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Check AMD GPU VRAM / busy % / hwmon temperature on a Linux host.",
            "Pick this over `system_gpu_nvidia` when the host has an AMD card and the `amdgpu` driver.",
        ]
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        use crate::skills::SkillCapability;
        #[cfg(target_os = "linux")]
        {
            if linux_drm_has_vendor(VENDOR_AMD) {
                return SkillCapability::Ready;
            }
            SkillCapability::unavailable(
                "no AMD GPU found in /sys/class/drm (PCI vendor 0x1002)",
                "needs an AMD GPU with the `amdgpu` kernel driver loaded",
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            SkillCapability::unavailable(
                "AMD GPU reads use the Linux DRM sysfs; this host is not Linux",
                "run on Linux with the `amdgpu` driver, or use `system_gpu_nvidia` instead",
            )
        }
    }
}

pub struct SystemGpuIntel;
impl Skill for SystemGpuIntel {
    fn name(&self) -> &'static str {
        "system_gpu_intel"
    }
    fn description(&self) -> &'static str {
        "Report Intel GPU stats (model, frequency, hwmon temperature) by reading the Linux DRM \
        sysfs nodes the `i915` / `xe` kernel driver publishes under `/sys/class/drm/card*/device/`. \
        Linux-only. Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move { Ok(text_result(blocking(gather_intel_gpu).await?)) })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[SkillExample {
            title: "Read Intel GPU stats (Linux)",
            args: r#"{}"#,
            note: Some(
                "Reads `/sys/class/drm/card*/device/` from the i915 / xe driver; Linux-only.",
            ),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Check Intel GPU frequency and hwmon temperature on a Linux host.",
            "Pick this over `system_gpu_nvidia` when the host's only GPU is an Intel iGPU/Arc.",
        ]
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        use crate::skills::SkillCapability;
        #[cfg(target_os = "linux")]
        {
            if linux_drm_has_vendor(VENDOR_INTEL) {
                return SkillCapability::Ready;
            }
            SkillCapability::unavailable(
                "no Intel GPU found in /sys/class/drm (PCI vendor 0x8086)",
                "needs an Intel GPU with the `i915` or `xe` kernel driver loaded",
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            SkillCapability::unavailable(
                "Intel GPU reads use the Linux DRM sysfs; this host is not Linux",
                "run on Linux with the `i915` / `xe` driver, or use `system_gpu_nvidia` instead",
            )
        }
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
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Read /etc/os-release",
                args: r#"{}"#,
                note: Some("Parses KEY=VALUE pairs (NAME, VERSION, ID, PRETTY_NAME, …). Says so on non-Linux hosts."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Identify the Linux distribution and version of the host.",
            "Branch behavior (package manager, paths) on `ID` / `ID_LIKE`.",
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(SystemInfo),
        Box::new(SystemDisks),
        Box::new(SystemGpuNvidia),
        Box::new(SystemGpuAmd),
        Box::new(SystemGpuIntel),
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
