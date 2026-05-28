# duckduckgo

|  |  |
| --- | --- |
| **Family** | engine (`HtmlEngineProvider` + `EngineSpec`) |
| **Kinds** | `web`, `code` |
| **Default-on** | yes (web + code) |
| **Keyless** | yes |
| **Render** | honored per call (`render=true`) |
| **Code** | [`src/providers/engine/duckduckgo.rs`](../../src/providers/engine/duckduckgo.rs) · shared: [`engine/mod.rs`](../../src/providers/engine/mod.rs) |
| **Config** | [`config/providers/duckduckgo.toml`](../../config/providers/duckduckgo.toml) (no tunables) |

## Why
DuckDuckGo's `lite` endpoint is a clean, keyless, mostly-static HTML surface — an
ideal default web/code source with no JS required. It supports the `site:`
operator, so code search can be scoped precisely to a forge. Its main weakness is
aggressive per-IP rate-limiting, so it is paired with the more automation-tolerant
[`mojeek`](mojeek.md) as a fallback.

## Features
- **Web search** over `lite.duckduckgo.com` (POST form), parsed from two CSS
  selectors.
- **Code search** by `site:`-scoping the query to the forges in `[code].sites`
  (`CodeScope::SiteOperator`) — precise, no post-filtering.
- **Render fallback:** `render=true` reissues the same query through the shared
  headless browser, which can slip past IP rate-limiting.

## Skills (tools)
- **General:** part of `web_search` / `code_search` whenever `duckduckgo` is in
  the relevant list in [`config/02-search.toml`](../../config/02-search.toml).
- **Per-provider:** `web_duckduckgo`, `code_duckduckgo` — target this engine
  alone (args `query`, `max_results?`, `language?`, `render?`).

## Schema / structs
Declares an [`EngineSpec`](../../src/providers/engine/mod.rs); the shared
`HtmlEngineProvider` does the rest:

```rust
EngineSpec {
    id: "duckduckgo",
    url: "https://lite.duckduckgo.com/lite/",
    method: Method::PostForm,
    extract: Extract::Selectors { link: "a.result-link", snippet: "td.result-snippet" },
    code_scope: CodeScope::SiteOperator,
    extra_params: &[],
}
```

Shared types: `SearchProvider` trait, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
