# hex

|  |  |
| --- | --- |
| **Family** | registry (`RegistryProvider` + `RegistrySpec`) |
| **Kind** | `docs` |
| **Default-on** | no — opt-in (add `"hex"` to `[providers].docs`) |
| **Keyless** | yes |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/registry/hex.rs`](../../src/providers/registry/hex.rs) · shared: [`registry/mod.rs`](../../src/providers/registry/mod.rs) |
| **Config** | [`config/providers/hex.toml`](../../config/providers/hex.toml) (no tunables) |

## Why
Hex is the Elixir/Erlang package index, with a keyless JSON search — the
authoritative way to find a Hex package.

## Features
- Keyless search of `hex.pm/api/packages?search=` (with `per_page` = the limit).
- The response is a **top-level array**; each package's description is nested at
  `/meta/description`, and the URL is built as `https://hex.pm/packages/<name>`.

## Skills (tools)
- **General:** part of `docs_search` when `hex` is in
  [`config/02-search.toml`](../../config/02-search.toml) (`[providers].docs`).
- **Per-provider:** `docs_hex` (args `query`, `max_results?`).

## Schema / structs
Declares a [`RegistrySpec`](../../src/providers/registry/mod.rs):

```rust
RegistrySpec {
    id: "hex",
    url: "https://hex.pm/api/packages",
    query_key: "search",
    size_key: Some("per_page"),
    extra_params: &[],
    results_ptr: "",                 // top-level array
    item: ItemMap {
        name: "/name",
        description: "/meta/description",
        url_field: None,
        url_template: Some("https://hex.pm/packages/{name}"),
        url_base: "",
        version: None,
    },
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
