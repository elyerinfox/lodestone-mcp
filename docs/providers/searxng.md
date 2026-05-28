# searxng

|  |  |
| --- | --- |
| **Family** | bespoke (unique transport/parse) |
| **Kinds** | `web`, `code` |
| **Default-on** | no — opt-in (set `[searxng].url`, then add `"searxng"` to the lists) |
| **Keyless** | yes (your instance; no account) |
| **Render** | not used (JSON API) |
| **Code** | [`src/providers/bespoke/searxng.rs`](../../src/providers/bespoke/searxng.rs) |
| **Config** | [`config/providers/searxng.toml`](../../config/providers/searxng.toml) → `[searxng].url` |

## Why
[SearXNG](https://docs.searxng.org/) is a self-hostable metasearch engine that
aggregates dozens of upstream engines behind one keyless JSON API. For users
willing to run an instance it gives far broader, higher-quality results than the
built-in DuckDuckGo+Mojeek pair — so it's the strongest keyless option, but
strictly opt-in (it depends on infrastructure you provide). It's bespoke because
its transport is a private JSON API, not an HTML results page.

## Features
- **Web + code search** against `{url}/search?format=json`.
- **Code mode** `site:`-scopes the query to `[code].sites` (same as the HTML
  engines) and attaches repo/path to results via the shared forge URL parser.
- Inactive until `[searxng].url` is set — a missing URL logs a warning and the
  provider is skipped, so the chain falls through.

## Requirements
The instance must enable JSON output (in its `settings.yml`):

```yaml
search:
  formats: [html, json]
```

## Skills (tools)
- **General:** part of `web_search` / `code_search` once `searxng` is added to a
  list in [`config/02-search.toml`](../../config/02-search.toml).
- **Per-provider:** `web_searxng`, `code_searxng` (args `query`, `max_results?`,
  `language?`).

## Schema / structs
A small provider struct holding the instance URL and the kind it serves
([`bespoke/searxng.rs`](../../src/providers/bespoke/searxng.rs)):

```rust
pub(crate) struct Searxng { base_url: String, kind: ProviderKind }

// SearchProvider::search:
//   GET {base_url}/search?q=…&format=json   (code: q is site:-scoped)
//   parse results[] → SearchResult { title, url, snippet }, then finish(kind, …)
```

Config:

```toml
[searxng]
url = ""   # base URL of your instance; empty = disabled. Env: LODESTONE_SEARXNG_URL
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
