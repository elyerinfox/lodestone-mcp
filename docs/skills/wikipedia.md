# Wikipedia — `wikipedia_search` / `wikipedia_summary`

|  |  |
| --- | --- |
| **Module** | [`src/skills/wikipedia.rs`](../../src/skills/wikipedia.rs) |
| **Tools** | `wikipedia_search`, `wikipedia_summary` |
| **Network** | keyless API |
| **Default** | on |
| **Config** | none (gate via `[tools]` in [`config/01-tools.toml`](../../config/01-tools.toml)) |

## What it does
Searches and reads Wikipedia with no account or key. `wikipedia_search` runs a
full-text search via the MediaWiki API; `wikipedia_summary` returns a page's lead
extract via the REST API, or the full plain-text article with `full=true`. Any
language edition is selectable through `lang` (default `en`); the code is sanitized
to a safe subdomain. Summary results are cached.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `wikipedia_search` | `query`, `lang?`, `max_results?` | Full-text search of Wikipedia. Returns matching article titles, a snippet, and the URL. `lang` selects the edition (e.g. "en", "de", "ja"; default "en"). Default 8 results, capped at 25. |
| `wikipedia_summary` | `title`, `lang?`, `full?`, `max_chars?` | Read an article: the lead summary by default, or the full plain-text article with `full=true` (truncated to `max_chars`). `title` is an article title like "Linux" or "Rust (programming language)"; `lang` selects the edition (default "en"). |

## Configuration & gating
No configuration. Both tools are on by default with no tunables; disable them in
`[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)). The `lang`
argument is per-call, not a config setting.

## Example uses
- **Look up then read a topic** — `wikipedia_search` for "rust language" to find the article title, then `wikipedia_summary` with `title="Rust (programming language)"` for the lead summary.
- **Read a full article** — `wikipedia_summary` with `title="Linux"`, `full=true`, and a large `max_chars` for the complete plain-text page.
- **Use another language edition** — `wikipedia_summary` with `title="Linux"`, `lang="de"` for the German lead summary.

## See also
- [tools.md](../tools.md) — full tool reference (Retrieve section).
- [kernel.md](kernel.md) — current Linux kernel releases from kernel.org.
