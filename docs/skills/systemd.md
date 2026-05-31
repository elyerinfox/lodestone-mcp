# systemd — `systemd_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/systemd.rs`](../../src/skills/systemd.rs) |
| **Tools** | `systemd_list`, `systemd_status`, `systemd_show`, `systemd_start`, `systemd_stop`, `systemd_restart` |
| **Network** | none — shells out to `systemctl` |
| **Default** | **off** — gated by `[systemd]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[systemd].enabled` / `allow_destructive` via `LODESTONE_SYSTEMD_*`. Defaults in [`src/config.rs`](../../src/config.rs). |
| **Platform** | Linux only |

## What it does

Linux service control via the local `systemctl`. Read tools (`list` / `status`
/ `show`) run freely; start / stop / restart confirm at call time unless
`[systemd].allow_destructive` pre-authorizes.

## Tools

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `systemd_list` | `state?`, `pattern?` | read | List units. `state`: `active` / `failed` / `loaded` / …. `pattern` matches a glob (e.g. `"nginx*"`). |
| `systemd_status` | `unit` | read | `systemctl status` for one unit. |
| `systemd_show` | `unit`, `properties?` | read | `systemctl show` — all unit properties, or a comma-separated subset. |
| `systemd_start` | `unit`, `confirm?`, `trust?` | **destructive** | Start a unit. Guarded. |
| `systemd_stop` | `unit`, `confirm?`, `trust?` | **destructive** | Stop a unit. Guarded. |
| `systemd_restart` | `unit`, `confirm?`, `trust?` | **destructive** | Restart a unit. Guarded. |

## Example uses

- **What's failing?** — `systemd_list { state: "failed" }`.
- **Drill in** — `systemd_status { unit: "nginx.service" }`.
- **Restart after config change** —
  `systemd_restart { unit: "nginx.service" }` → token →
  `systemd_restart { unit: "nginx.service", confirm: "<token>" }`.

## Notes

- **Linux only.** On macOS / Windows the tools return a clear platform error.
- **No `enable` / `disable` / `mask`.** Persistent state changes are out of
  scope for v1; if you need them, do them through `shell_run`.

## See also

- [tools.md](../tools.md)
- [skills/docker.md](docker.md) / [skills/kubernetes.md](kubernetes.md) — for
  containers / clusters.
- [skills/shell.md](shell.md) — escape hatch for systemctl verbs not exposed
  here.
