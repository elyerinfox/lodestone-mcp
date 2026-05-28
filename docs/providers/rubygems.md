# rubygems

|  |  |
| --- | --- |
| **Family** | registry (`RegistryProvider` + `RegistrySpec`) |
| **Kind** | `docs` |
| **Default-on** | no — opt-in (add `"rubygems"` to `[providers].docs`) |
| **Keyless** | yes |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/registry/rubygems.rs`](../../src/providers/registry/rubygems.rs) · shared: [`registry/mod.rs`](../../src/providers/registry/mod.rs) |
| **Config** | [`config/providers/rubygems.toml`](../../config/providers/rubygems.toml) (no tunables) |

## Why
RubyGems is the Ruby package index, with a keyless JSON search endpoint — the
authoritative way to find a Ruby gem.

## Features
- Keyless search of `rubygems.org/api/v1/search.json?query=`.
- The response is a **top-level array** (handled by the empty/root results
  pointer); each gem becomes `name version` → `https://rubygems.org/gems/<name>`
  with its `info` text as the snippet.

## Skills (tools)
- **General:** part of `docs_search` when `rubygems` is in
  [`config/02-search.toml`](../../config/02-search.toml) (`[providers].docs`).
- **Per-provider:** `docs_rubygems` (args `query`, `max_results?`).

## Schema / structs
Declares a [`RegistrySpec`](../../src/providers/registry/mod.rs):

```rust
RegistrySpec {
    id: "rubygems",
    url: "https://rubygems.org/api/v1/search.json",
    query_key: "query",
    size_key: None,
    extra_params: &[],
    results_ptr: "",                 // top-level array
    item: ItemMap {
        name: "/name",
        description: "/info",
        url_field: None,
        url_template: Some("https://rubygems.org/gems/{name}"),
        url_base: "",
        version: Some("/version"),
    },
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
