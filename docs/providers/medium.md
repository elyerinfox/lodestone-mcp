# medium

|  |  |
| --- | --- |
| **Family** | bespoke (unique transport/parse) |
| **Kind** | `web` |
| **Default-on** | no — opt-in (add `"medium"` to `[providers].web`) |
| **Keyless** | yes |
| **Render** | not used (RSS feed, not HTML scraping) |
| **Code** | [`src/providers/bespoke/medium.rs`](../../src/providers/bespoke/medium.rs) |
| **Config** | [`config/providers/medium.toml`](../../config/providers/medium.toml) (no tunables) |

## Why
Medium hosts a lot of practical engineering write-ups, but its search page is
JS/bot-walled and not keyless-scrapeable. Its **per-tag RSS feeds**, however, are
keyless and stable — so this provider treats the query as a Medium **tag** and
returns recent articles from `medium.com/feed/tag/<tag>`. It's bespoke because the
transport is RSS/XML, not an HTML search results page.

## Features
- Slugifies the query into a tag (lowercase alphanumerics joined by hyphens) and
  fetches that tag's RSS feed.
- Parses `<item>` entries into title/link/snippet (description excerpt, truncated).

## Caveats
This surfaces **recent posts for a topic**, not full-text relevance search — best
as a supplementary `web` source. Not in the default list — add `"medium"` to opt
in.

## Skills (tools)
- **General:** part of `web_search` when `medium` is in `[providers].web`.
- **Per-provider:** `web_medium` (args `query`, `max_results?`).

## Schema / structs
A plain unit-struct provider
([`bespoke/medium.rs`](../../src/providers/bespoke/medium.rs)):

```rust
pub(crate) struct Medium;

// SearchProvider::search:
//   tag = tag_slug(query)                       // "rust async" → "rust-async"
//   GET medium.com/feed/tag/<tag>  (Accept: application/rss+xml)
//   parse <item> → SearchResult { title, url, snippet }
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
