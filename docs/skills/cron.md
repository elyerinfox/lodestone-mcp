# cron — describe / next / validate

|  |  |
| --- | --- |
| **Module** | [`src/skills/cron_expr.rs`](../../src/skills/cron_expr.rs) |
| **Tools** | `cron_describe`, `cron_next`, `cron_validate` |
| **Network** | none — pure local compute |
| **Default** | on (no config gate) |

## What it does

Three tools covering cron expressions:

- **`cron_describe { expression, timezone? }`** — plain-English description
  plus the next 3 firings when the expression iterates. Calls out the
  Vixie cron DOM/DOW OR rule explicitly when both fields are restricted,
  because that's the trap LLMs hit most often. Accepts both 5-field
  (`min hour dom month dow`) and 6-field (`sec min hour dom month dow`)
  form; a 5-field expression has seconds=0 synthesized for the
  underlying parser.
- **`cron_next { expression, count?, from?, timezone? }`** — list the next
  N firings as ISO timestamps. Default `count` is 5 (max 100), default
  `from` is now, default timezone is UTC.
- **`cron_validate { expression }`** — parse and report `valid=true` or a
  precise error pointing at the bad field.

## DOM / DOW semantics — the LLM trap

When both day-of-month and day-of-week are restricted (i.e. neither is
`*`), Vixie cron uses **OR** — the entry fires when EITHER constraint
matches. `0 0 13 * 5` fires every Friday AND on the 13th of every month,
not only on Friday-the-13th. `cron_describe` calls this out in its
output. The underlying `cron` crate is Quartz-flavored and will refuse
to iterate Vixie expressions that restrict both DOM and DOW (it requires
one to be `?`); `cron_describe` handles that gracefully by still
producing the English description and surfacing an `iteration_note`.

## Sources

- `man 5 crontab` (Vixie cron format).
- POSIX `crontab` definition (IEEE Std 1003.1-2024).

## Example flow

```
1. cron_validate { expression: "0 0 13 * 5" }
   → valid=true

2. cron_describe { expression: "0 0 13 * 5" }
   → english: "at 0:00, on day-of-month 13 on weekday(s) 5 [0=Sun]. NOTE: Vixie cron OR-s DOM and DOW…"

3. cron_next { expression: "0 9 * * 1-5", count: 3 }
   → 3 upcoming weekday 09:00 UTC timestamps
```

## See also

- [`docs/golden-rules.md`](../golden-rules.md) — golden rule 1 (keyless),
  golden rule 12 (citations).
