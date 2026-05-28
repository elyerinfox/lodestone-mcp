# cratesio

|  |  |
| --- | --- |
| **Family** | registry (`RegistryProvider` + `RegistrySpec`) |
| **Kind** | `docs` |
| **Default-on** | yes (docs) |
| **Keyless** | yes |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/registry/cratesio.rs`](../../src/providers/registry/cratesio.rs) · shared: [`registry/mod.rs`](../../src/providers/registry/mod.rs) |
| **Config** | [`config/providers/cratesio.toml`](../../config/providers/cratesio.toml) (no tunables) |

## Why
crates.io is the canonical Rust package index, with a clean, keyless JSON search
API — the authoritative way to find a Rust crate (far better than scraping a web
engine for it). It's a `docs`-kind provider so package/library lookups don't get
mixed into web or code-file search.

## Features
- Keyless search of `crates.io/api/v1/crates?q=` (with `per_page` = the limit).
- Each hit becomes `name version` → `https://crates.io/crates/<name>` with the
  crate description as the snippet and the newest version in `meta`.

## Skills (tools)
- **General:** part of `docs_search` when `cratesio` is in
  [`config/02-search.toml`](../../config/02-search.toml) (`[providers].docs`).
- **Per-provider:** `docs_cratesio` (args `query`, `max_results?`).

## Schema / structs
Declares a [`RegistrySpec`](../../src/providers/registry/mod.rs):

```rust
RegistrySpec {
    id: "cratesio",
    url: "https://crates.io/api/v1/crates",
    query_key: "q",
    size_key: Some("per_page"),
    extra_params: &[],
    results_ptr: "/crates",
    item: ItemMap {
        name: "/name",
        description: "/description",
        url_field: None,
        url_template: Some("https://crates.io/crates/{name}"),
        url_base: "",
        version: Some("/newest_version"),
    },
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
