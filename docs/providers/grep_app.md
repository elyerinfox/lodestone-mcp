# grep_app

|  |  |
| --- | --- |
| **Family** | bespoke (unique transport/parse) |
| **Kind** | `code` |
| **Default-on** | yes (code; first in the chain) |
| **Keyless** | yes |
| **Render** | not used (JSON API, not HTML scraping) |
| **Code** | [`src/providers/bespoke/grep_app.rs`](../../src/providers/bespoke/grep_app.rs) |
| **Config** | [`config/providers/grep_app.toml`](../../config/providers/grep_app.toml) (no tunables) |

## Why
[grep.app](https://grep.app) indexes the literal source of huge numbers of public
repos and exposes a JSON search endpoint, giving **true substring/code matches**
(not just page-title relevance) — a much stronger first hop for code search than a
general web engine. It's bespoke because it neither fits the engine HTML-scrape
shape nor the forge site-search shape: it speaks a private JSON API.

## Features
- Queries `grep.app/api/search` and builds `github.com/<repo>/blob/<branch>/<path>`
  URLs from the hits, with the matched code as the snippet.
- Degrades gracefully: grep.app is frequently behind a bot-challenge that returns
  HTML instead of JSON — when that happens the provider returns **no results**
  (not an error), so the registry simply falls through to the next code provider.

## Skills (tools)
- **General:** part of `code_search` (default-enabled, tried first).
- **Per-provider:** `code_grep_app` (args `query`, `max_results?`).

## Schema / structs
A plain unit-struct provider
([`bespoke/grep_app.rs`](../../src/providers/bespoke/grep_app.rs)):

```rust
pub(crate) struct GrepApp;

// SearchProvider::search → GET grep.app/api/search?q=…  (Accept: application/json)
//   non-2xx        → Ok(vec![])   (fall through)
//   non-JSON body  → Ok(vec![])   (bot-challenge HTML)
//   else           → parse /hits/hits[] into SearchResult { repo, path, url, snippet }
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
