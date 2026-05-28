# mdn

|  |  |
| --- | --- |
| **Family** | registry (`RegistryProvider` + `RegistrySpec`) |
| **Kind** | `docs` |
| **Default-on** | yes (docs) |
| **Keyless** | yes |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/registry/mdn.rs`](../../src/providers/registry/mdn.rs) · shared: [`registry/mod.rs`](../../src/providers/registry/mod.rs) |
| **Config** | [`config/providers/mdn.toml`](../../config/providers/mdn.toml) (no tunables) |

## Why
[MDN Web Docs](https://developer.mozilla.org) is the reference for web platform
APIs (JavaScript, CSS, HTML, Web APIs). Its keyless JSON search returns the
canonical doc page for a topic — much better than a general web search for "how
does `Array.prototype.map` work". Serves the `docs` kind.

## Features
- Keyless search of `developer.mozilla.org/api/v1/search?q=`.
- Each document's `mdn_url` is site-relative, so it's prefixed with the MDN
  origin; the page title is the result title and `summary` the snippet.

## Skills (tools)
- **General:** part of `docs_search` when `mdn` is in
  [`config/02-search.toml`](../../config/02-search.toml) (`[providers].docs`).
- **Per-provider:** `docs_mdn` (args `query`, `max_results?`).

## Schema / structs
Declares a [`RegistrySpec`](../../src/providers/registry/mod.rs) using `url_base`
to absolutize the relative `mdn_url`:

```rust
RegistrySpec {
    id: "mdn",
    url: "https://developer.mozilla.org/api/v1/search",
    query_key: "q",
    size_key: None,
    extra_params: &[],
    results_ptr: "/documents",
    item: ItemMap {
        name: "/title",
        description: "/summary",
        url_field: Some("/mdn_url"),
        url_template: None,
        url_base: "https://developer.mozilla.org",
        version: None,
    },
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
