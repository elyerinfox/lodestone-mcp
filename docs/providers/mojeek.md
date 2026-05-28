# mojeek

|  |  |
| --- | --- |
| **Family** | engine (`HtmlEngineProvider` + `EngineSpec`) |
| **Kinds** | `web`, `code` |
| **Default-on** | yes (web + code) |
| **Keyless** | yes |
| **Render** | honored per call (`render=true`) |
| **Code** | [`src/providers/engine/mojeek.rs`](../../src/providers/engine/mojeek.rs) · shared: [`engine/mod.rs`](../../src/providers/engine/mod.rs) |
| **Config** | [`config/providers/mojeek.toml`](../../config/providers/mojeek.toml) (no tunables) |

## Why
Mojeek runs its own independent crawl/index (not a Bing/Google reseller) and is
unusually tolerant of automated requests, which makes it the reliable **fallback**
behind [`duckduckgo`](duckduckgo.md). It has no `site:` operator, so code search
falls back to keyword-scoping plus result filtering.

## Features
- **Web search** over `www.mojeek.com/search` (GET), parsed from two CSS
  selectors.
- **Code search** via `CodeScope::Keyword`: the forge domains are appended as
  keywords, more results are fetched, and hits are filtered to those domains.
- **Render fallback:** `render=true` runs the query through the shared headless
  browser.

## Skills (tools)
- **General:** part of `web_search` / `code_search` whenever `mojeek` is in the
  relevant list in [`config/02-search.toml`](../../config/02-search.toml). Also
  invoked directly by the forge providers as their second-chance search.
- **Per-provider:** `web_mojeek`, `code_mojeek` (args `query`, `max_results?`,
  `language?`, `render?`).

## Schema / structs
Declares an [`EngineSpec`](../../src/providers/engine/mod.rs):

```rust
EngineSpec {
    id: "mojeek",
    url: "https://www.mojeek.com/search",
    method: Method::Get,
    extract: Extract::Selectors { link: "a.title", snippet: "p.s" },
    code_scope: CodeScope::Keyword,
    extra_params: &[],
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
