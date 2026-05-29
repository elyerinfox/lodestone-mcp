# Search — `web_search` / `code_search` / `docs_search` / `qa_search`

|  |  |
| --- | --- |
| **Module** | [`src/skills/search.rs`](../../src/skills/search.rs) |
| **Tools** | `web_search`, `code_search`, `docs_search`, `qa_search`, `qa_stackoverflow_answers`, plus one `<kind>_<id>` tool per configured provider |
| **Network** | keyless web scrape / public API (`render=true` routes through the headless browser) |
| **Default** | on |
| **Config** | [`config/02-search.toml`](../../config/02-search.toml) (`[providers]`, `[search]`), [`config/providers/stackexchange.toml`](../../config/providers/stackexchange.toml) (`[stackexchange]`) |

## What it does
The four general search tools run the provider [`Registry`](../../src/provider.rs) for a given kind (web, code, docs, Q&A), combining the configured providers per `[search].strategy` and returning a ranked, date-stamped list of title / URL / snippet. Each kind also gets auto-generated per-provider tools named `<kind>_<id>` (e.g. `web_mojeek`, `code_github`, `qa_stackoverflow`) that target a single source and bypass the chain/strategy. `qa_stackoverflow_answers` reads a question's body and top answers from the StackExchange network.

## Tools
| Tool | Arguments | Purpose |
| --- | --- | --- |
| `web_search` | `query`, `max_results?`, `render?` | General web search across `[providers].web` (DuckDuckGo, Mojeek, …). Default 8 results, cap 25. |
| `code_search` | `query`, `language?`, `max_results?`, `render?` | Source-code search across `[providers].code` (grep.app, GitHub-scoped web). Default 10, cap 25. |
| `docs_search` | `query`, `max_results?`, `render?` | Package registries (crates.io, npm, MDN) and framework/tooling docs. Default 10, cap 25. `render` only affects doc-site searches; JSON registries ignore it. |
| `qa_search` | `query`, `site?`, `max_results?`, `render?` | Search StackExchange Q&A; `site` defaults to the configured `default_site`. Default 8, cap 25. `render` scrapes via the browser (no API quota). |
| `qa_stackoverflow_answers` | `question`, `site?`, `max_answers?`, `render?` | Read a question's body + top answers (by votes, with code) from a URL or numeric id. Default 3 answers, cap 10. `render=true` scrapes the page instead of the API (saves quota; `stackoverflow` site only — other sites fall back to the API). |
| `<kind>_<id>` | `query`, `max_results?`, `language?`, `site?`, `render?` | Query one configured provider directly. Default 10, cap 25. |

## Configuration & gating
Provider lists per kind live in [`config/02-search.toml`](../../config/02-search.toml) under `[providers]` (`web`, `code`, `qa`, `docs`); `[search].strategy` controls how a kind's providers are combined, `[search].ranking` how hits are ordered. The per-provider `<kind>_<id>` tools are generated from exactly the configured providers. StackExchange behaviour is set in [`config/providers/stackexchange.toml`](../../config/providers/stackexchange.toml) `[stackexchange]`: `default_site` (env `LODESTONE_STACKEXCHANGE_SITE`), an optional `key` that only raises the per-IP quota (env `LODESTONE_STACKEXCHANGE_KEY`), and an `allowed_sites` allowlist (env `LODESTONE_STACKEXCHANGE_ALLOWED_SITES`) — a `qa_search`/`qa_stackoverflow_answers` call for a site outside a non-empty allowlist is rejected. Each tool is individually gateable via `[tools]`. Answer/search results are cached, clamped to the server's `[retrieval].max_chars`.

## Example uses
- **Find how a library is used in the wild** — `code_search` (e.g. `query` a symbol, `language: "rust"`) then `fetch_repo_file` on a result URL to read the full source.
- **Read a package's API docs** — `docs_search` for the crate/package name, then `fetch_page` on the docs URL.
- **Resolve a specific error** — `qa_search` for the error text (StackOverflow), then `qa_stackoverflow_answers` with the chosen question URL to read the accepted/top answers and code, optionally `render=true` to dodge the API quota.
- **Target one engine** — call `web_mojeek` directly when DuckDuckGo is rate-limited, or set `render=true` on `web_search` to fetch through a real headless browser past a bot-wall.

## See also
[tools.md](../tools.md), [retrieve.md](retrieve.md), [archive.md](archive.md), [providers/stackexchange.md](../providers/stackexchange.md), [providers/cratesio.md](../providers/cratesio.md)
