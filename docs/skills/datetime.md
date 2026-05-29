# Date & time — `datetime` / `date_diff` / `time_convert`

|  |  |
| --- | --- |
| **Module** | [`src/skills/datetime.rs`](../../src/skills/datetime.rs) |
| **Tools** | `datetime`, `date_diff`, `time_convert` |
| **Network** | local-only |
| **Default** | on |
| **Config** | none |

## What it does
The model's training data has no current time, so these read the system clock and do
timezone/interval arithmetic — pure local computation (chrono + chrono-tz), no network.
`datetime` reports "now", `date_diff` measures the gap between two dates, and
`time_convert` re-expresses an instant in another IANA timezone.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `datetime` | `timezone?` | Current date/time — local (with UTC offset + weekday), UTC, and Unix timestamp; `timezone` (IANA name) adds that zone too. |
| `date_diff` | `from`, `to?` | Difference between two dates: days (≈years past 365), hours, and a human "ago / from now"; `to` defaults to now. |
| `time_convert` | `time`, `to_tz`, `from_tz?` | Convert a date/time to the `to_tz` IANA zone; `from_tz` (default UTC) interprets offset-less inputs. |

Inputs accept a Unix timestamp (seconds), RFC3339 with an offset, an ISO
`YYYY-MM-DD[ T]HH:MM[:SS]`, or a bare `YYYY-MM-DD` (treated as UTC midnight).
Timezones are IANA names such as `America/New_York`, `Asia/Tokyo`, or `UTC`.

## Configuration & gating
No configuration. Each tool is independently gateable via `[tools]`.

## Example uses
- **Anchor "now"** — call `datetime` first whenever recency matters, then reason about it.
- **Judge recency** — `date_diff` from a release date to now to see how old a version is.
- **Cross-zone scheduling** — `time_convert` a meeting time from `from_tz` into `Asia/Tokyo`.

## See also
[tools.md](../tools.md)
