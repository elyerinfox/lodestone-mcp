# System information — `system_info` / `system_disks` / `system_gpu`

|  |  |
| --- | --- |
| **Module** | [`src/skills/sysinfo.rs`](../../src/skills/sysinfo.rs) |
| **Tools** | `system_info`, `system_disks`, `system_gpu` |
| **Network** | local-only (host) |
| **Default** | on — gated by `[sysinfo]` (`enabled`) |
| **Config** | `[sysinfo]` in [`config/13-sysinfo.toml`](../../config/13-sysinfo.toml) |

## What it does
Reports read-only facts about the host the server runs on: OS/kernel, CPU, memory,
disks, and (where present) NVIDIA GPU stats. Purely read-only, so it is on by
default; there are no destructive tools and no confirmation guard. Cross-platform via
the `sysinfo` crate (Linux reads `/proc`/`/sys`, Windows uses the OS APIs). GPU stats
use NVML, loaded at runtime — when the NVML library/driver is absent `system_gpu`
returns a clear message rather than failing.

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `system_info` | — | read | Host name, OS/kernel, uptime, CPU (model, logical cores, frequency, usage), memory and swap usage. |
| `system_disks` | — | read | Mounted disks/volumes: device, filesystem, mount point, total/used/free space. |
| `system_gpu` | — | read | NVIDIA GPU name, memory, utilization, and temperature via NVML; a clear message if no NVIDIA GPU / NVML library is present. |

All three take no arguments. Blocking work (a sysinfo refresh plus a short CPU
sampling interval, NVML queries) runs off the async runtime.

## Configuration & gating
The single key is `[sysinfo].enabled` (env `LODESTONE_SYSINFO_ENABLED`). It is on by
default since the tools are read-only; set `enabled = false` to hide all three.
There is nothing destructive here, so no guard, `confirm`/`trust`, or
`allow_destructive` applies.

## Example uses
- **Check host health** — `system_info` for CPU/memory load, then `system_disks` to confirm free space before a large download or build.
- **Confirm GPU availability** — `system_gpu` to see whether an NVIDIA GPU and NVML are present (and current utilization/temperature).
- **Capacity snapshot** — `system_disks` to list every mounted volume and its free space.

## See also
[golden-rules.md](../golden-rules.md), [tools.md](../tools.md)
