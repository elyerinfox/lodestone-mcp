# packagist

|  |  |
| --- | --- |
| **Family** | registry (`RegistryProvider` + `RegistrySpec`) |
| **Kind** | `docs` |
| **Default-on** | no — opt-in (add `"packagist"` to `[providers].docs`) |
| **Keyless** | yes |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/registry/packagist.rs`](../../src/providers/registry/packagist.rs) · shared: [`registry/mod.rs`](../../src/providers/registry/mod.rs) |
| **Config** | [`config/providers/packagist.toml`](../../config/providers/packagist.toml) (no tunables) |

## Why
Packagist is the PHP/Composer package index, with a keyless JSON search — the
authoritative way to find a Composer package.

## Features
- Keyless search of `packagist.org/search.json?q=` (with `per_page` = the limit).
- Results live under `/results`; each carries a ready `url`, with the package
  description as the snippet.

## Skills (tools)
- **General:** part of `docs_search` when `packagist` is in
  [`config/02-search.toml`](../../config/02-search.toml) (`[providers].docs`).
- **Per-provider:** `docs_packagist` (args `query`, `max_results?`).

## Schema / structs
Declares a [`RegistrySpec`](../../src/providers/registry/mod.rs):

```rust
RegistrySpec {
    id: "packagist",
    url: "https://packagist.org/search.json",
    query_key: "q",
    size_key: Some("per_page"),
    extra_params: &[],
    results_ptr: "/results",
    item: ItemMap {
        name: "/name",
        description: "/description",
        url_field: Some("/url"),
        url_template: Some("https://packagist.org/packages/{name}"),
        url_base: "",
        version: None,
    },
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
