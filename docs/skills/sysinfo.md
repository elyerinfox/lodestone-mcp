# System information — `system_info` / `system_disks` / `system_gpu_*` / `system_os_release`

|  |  |
| --- | --- |
| **Module** | [`src/skills/sysinfo.rs`](../../src/skills/sysinfo.rs) |
| **Tools** | `system_info`, `system_disks`, `system_gpu_nvidia`, `system_gpu_amd`, `system_gpu_intel`, `system_os_release` |
| **Network** | local-only (host) |
| **Default** | on — gated by `[sysinfo]` (`enabled`) |
| **Config** | `[sysinfo]` in [`config/13-sysinfo.toml`](../../config/13-sysinfo.toml) |

## What it does
Reports read-only facts about the host the server runs on: OS/kernel, CPU, memory,
disks, OS release (Linux), and GPU stats for each detected vendor (NVIDIA, AMD,
Intel). Purely read-only, so it is on by default; there are no destructive tools
and no confirmation guard. Cross-platform via the `sysinfo` crate (Linux reads
`/proc`/`/sys`, Windows uses the OS APIs).

GPU access is split **one tool per vendor** (golden rule 9) because the backends
are genuinely different methodologies — NVML is a userspace driver library; AMD
and Intel are read from kernel-published DRM sysfs nodes — and each has its own
per-tool capability check so an LLM can pick the available one without guessing.

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `system_info` | — | read | Host name, OS/kernel, uptime, CPU (model, logical cores, frequency, usage), memory and swap usage. |
| `system_disks` | — | read | Mounted disks/volumes: device, filesystem, mount point, total/used/free space. |
| `system_gpu_nvidia` | — | read | NVIDIA GPU name, memory, utilization, and temperature via NVML. Cross-platform (Windows + Linux); needs the NVIDIA driver's NVML library (`nvml.dll` / `libnvidia-ml.so`). |
| `system_gpu_amd` | — | read | AMD GPU model, VRAM totals, busy %, and hwmon temperature from `/sys/class/drm/card*/device/` published by the `amdgpu` kernel driver. Linux-only. |
| `system_gpu_intel` | — | read | Intel GPU model, frequency (current / max), and hwmon temperature from `/sys/class/drm/card*/device/` published by the `i915` / `xe` kernel driver. Linux-only. |
| `system_os_release` | — | read | Parse `/etc/os-release` (Linux distro identifier per the systemd spec). Returns `NAME`, `VERSION`, `ID`, `ID_LIKE`, `PRETTY_NAME`, `VERSION_ID`, `HOME_URL`, etc. Non-Linux hosts get a clear message. |

All take no arguments. Blocking work (sysinfo refresh, NVML init, sysfs reads)
runs off the async runtime.

### GPU capability gating
Each `system_gpu_*` tool runs its own per-tool `check_capability`:

- **`system_gpu_nvidia`** — `Ready` iff `Nvml::init()` succeeds. Otherwise
  `Unavailable` with a hint to install the NVIDIA driver / `nvidia-utils`.
- **`system_gpu_amd`** — `Ready` iff this host is Linux **and** at least one
  `/sys/class/drm/card[0-9]+/device/vendor` reads PCI vendor `0x1002`. Otherwise
  `Unavailable` (with a hint for non-Linux hosts about needing ADL / IOKit, which
  isn't integrated).
- **`system_gpu_intel`** — same as AMD with PCI vendor `0x8086`.

The capability state is surfaced to the LLM, the console, and the dashboard's
*Host capabilities* section, so the model sees exactly which vendor backends are
usable and doesn't waste a call.

## Configuration & gating
The single key is `[sysinfo].enabled` (env `LODESTONE_SYSINFO_ENABLED`). It is on by
default since the tools are read-only; set `enabled = false` to hide all of them.
Per-tool capability checks fire regardless and may downgrade individual GPU
tools to `Unavailable` even when the family is enabled (e.g. you'll see
`system_gpu_amd` as unavailable on a Windows host). There is nothing destructive
here, so no guard, `confirm`/`trust`, or `allow_destructive` applies.

## Example uses
- **Check host health** — `system_info` for CPU/memory load, then `system_disks` to confirm free space before a large download or build.
- **Confirm GPU availability** — call the `system_gpu_<vendor>` matching the host (the capability surface tells you which is `Ready`).
- **Distro detection** — `system_os_release` to discover the Linux distro family before recommending a package-manager command.
- **Capacity snapshot** — `system_disks` to list every mounted volume and its free space.

## See also
[golden-rules.md](../golden-rules.md), [tools.md](../tools.md)
