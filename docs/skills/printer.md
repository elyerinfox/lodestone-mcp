# Printers — `printer_list` / `printer_print`

|  |  |
| --- | --- |
| **Module** | [`src/skills/printer.rs`](../../src/skills/printer.rs) |
| **Tools** | `printer_list`, `printer_print` |
| **Network** | local hardware (OS print system) |
| **Default** | off — gated by `[printer]` |
| **Config** | `[printer]` in [`config/16-devices.toml`](../../config/16-devices.toml) |

## What it does
Lists the machine's printers and prints text through the OS print system. There is
no good cross-platform Rust printing crate, so this **shells out**: CUPS
`lpstat`/`lp` on Unix, PowerShell `Get-Printer`/`Out-Printer` on Windows. This is
**direct hardware access**, so the family is **off by default**. Printing is
side-effecting, so `printer_print` routes through the confirmation
[`guard`](../../src/skills/guard.rs).

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `printer_list` | — | read | List printers known to the print system (CUPS `lpstat -e` / Windows spooler `Get-Printer`). |
| `printer_print` | `text`, `printer?`, `confirm?`, `trust?` | **side-effecting** | Print `text`; `printer` is a name from `printer_list` (omit for the system default). Returns the char count sent (confirm first). |

## Configuration & gating
- `[printer].enabled` (default `false`, env `LODESTONE_PRINTER_ENABLED`) — exposes
  the `printer_*` tools. **Off by default**; an explicit grant is required, after
  which gating lives in `disabled_by_config`. Also gateable via `[tools]`. See
  [`config/16-devices.toml`](../../config/16-devices.toml).
- **Confirmation guard.** `printer_print` is side-effecting: the **first** call
  prints nothing and returns a one-time `confirm` token describing the job (char
  count + target printer). Call again with `confirm=<token>` to actually print, or
  `confirm=<token>` plus `trust=true` to stop being asked for `printer_print` for
  the rest of the session. Tokens are single-use and expire after 5 minutes. There
  is no pre-authorize flag for printer. `printer_list` is read-only.

## Example uses
- **See what's available** — `printer_list` to get printer names.
- **Print a note** — `printer_print text="Hello"` (returns a token; call again with
  `confirm=<token>`); add `printer="Office_Laser"` to target a specific one.

## See also
[tools.md](../tools.md), [golden-rules.md](../golden-rules.md)
