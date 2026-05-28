# npm

|  |  |
| --- | --- |
| **Family** | registry (`RegistryProvider` + `RegistrySpec`) |
| **Kind** | `docs` |
| **Default-on** | yes (docs) |
| **Keyless** | yes |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/registry/npm.rs`](../../src/providers/registry/npm.rs) · shared: [`registry/mod.rs`](../../src/providers/registry/mod.rs) |
| **Config** | [`config/providers/npm.toml`](../../config/providers/npm.toml) (no tunables) |

## Why
npm is the JavaScript/Node package index. Its registry exposes a keyless JSON
search endpoint — the authoritative way to find an npm package. Like crates.io it
serves the `docs` kind so package lookups stay separate from web/code search.

## Features
- Keyless search of `registry.npmjs.org/-/v1/search?text=` (with `size` = the
  limit).
- Each object nests its package under `/package`; results become `name version` →
  the package's `links.npm` URL (falling back to a `npmjs.com/package/<name>`
  template), with the description as the snippet.

## Skills (tools)
- **General:** part of `docs_search` when `npm` is in
  [`config/02-search.toml`](../../config/02-search.toml) (`[providers].docs`).
- **Per-provider:** `docs_npm` (args `query`, `max_results?`).

## Schema / structs
Declares a [`RegistrySpec`](../../src/providers/registry/mod.rs):

```rust
RegistrySpec {
    id: "npm",
    url: "https://registry.npmjs.org/-/v1/search",
    query_key: "text",
    size_key: Some("size"),
    extra_params: &[],
    results_ptr: "/objects",
    item: ItemMap {
        name: "/package/name",
        description: "/package/description",
        url_field: Some("/package/links/npm"),
        url_template: Some("https://www.npmjs.com/package/{name}"),
        url_base: "",
        version: Some("/package/version"),
    },
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
