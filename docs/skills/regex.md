# Regex — `regex_search` / `regex_replace`

|  |  |
| --- | --- |
| **Module** | [`src/skills/regex.rs`](../../src/skills/regex.rs) |
| **Tools** | `regex_search`, `regex_replace` |
| **Network** | local-only |
| **Default** | on |
| **Config** | none |

## What it does
Matches and rewrites text with regular expressions — local, no network. Uses the Rust
`regex` crate syntax (linear-time; **no** look-around or backreferences). `regex_search`
returns each match plus its numbered and named capture groups; `regex_replace`
substitutes matches, supporting `$1` / `${name}` group references in the replacement.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `regex_search` | `pattern`, `text`, `all?`, `ignore_case?` | Find matches and their capture groups; `all` defaults true (capped at 200 matches), `all=false` returns only the first. |
| `regex_replace` | `pattern`, `text`, `replacement`, `all?`, `ignore_case?` | Substitute matches; `replacement` supports `$1` / `${name}` refs; `all` defaults true, `all=false` replaces only the first. |

## Configuration & gating
No configuration. Each tool is independently gateable via `[tools]`. Invalid patterns
return a clear error.

## Example uses
- **Extract fields** — `regex_search` with named groups to pull IDs/dates out of log lines.
- **Bulk rewrite** — `regex_replace` to renumber or reformat references across a blob.
- **Case-insensitive scan** — `regex_search` with `ignore_case=true` to find all variants.

## See also
[tools.md](../tools.md)
