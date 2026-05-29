# Archive — `wayback_fetch`

|  |  |
| --- | --- |
| **Module** | [`src/skills/archive.rs`](../../src/skills/archive.rs) |
| **Tools** | `wayback_fetch` |
| **Network** | keyless public API (Internet Archive Wayback Machine) |
| **Default** | on |
| **Config** | [`config/00-server.toml`](../../config/00-server.toml) (`[retrieval]`) |

## What it does
`wayback_fetch` reads the closest archived snapshot of a URL from the Internet Archive Wayback Machine (keyless). It queries `archive.org/wayback/available`, resolves the snapshot to its raw, toolbar-free form, and returns the readable text via the shared [`fetch_readable`](../../src/skills/retrieve.rs) primitive (HTML stripped; PDFs text-extracted). Useful when a page is down, paywalled, changed, or blocking automated access, or to view a historical version. Output is truncated to a character budget and the result is cached.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `wayback_fetch` | `url`, `timestamp?`, `max_chars?` | Fetch the readable text of the closest archived snapshot of `url`. `timestamp` is `YYYYMMDD` or `YYYYMMDDhhmmss` to target a capture near a date (omit for the most recent). Returns an error if no snapshot exists. |

## Configuration & gating
No skill-specific configuration. Like the other retrieval tools it honors [`config/00-server.toml`](../../config/00-server.toml) `[retrieval]`: `default_chars` when `max_chars` is omitted (env `LODESTONE_RETRIEVAL_DEFAULT_CHARS`) and the `max_chars` hard cap (env `LODESTONE_RETRIEVAL_MAX_CHARS`) — pass a larger `max_chars` to get more text. Gateable via `[tools]`.

## Example uses
- **Recover a dead link** — `fetch_page` returns an error or empty body, so fall back to `wayback_fetch` on the same `url` to read the last archived copy.
- **Read a paywalled/changed article** — `web_search` for the source, then `wayback_fetch` on the result URL to get a readable archived version.
- **View a historical version** — call `wayback_fetch` with `timestamp` (e.g. `20180101`) to read the page as it stood near that date.

## See also
[tools.md](../tools.md), [retrieve.md](retrieve.md), [search.md](search.md)
