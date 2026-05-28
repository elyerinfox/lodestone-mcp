# nuget

|  |  |
| --- | --- |
| **Family** | registry (`RegistryProvider` + `RegistrySpec`) |
| **Kind** | `docs` |
| **Default-on** | no — opt-in (add `"nuget"` to `[providers].docs`) |
| **Keyless** | yes |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/registry/nuget.rs`](../../src/providers/registry/nuget.rs) · shared: [`registry/mod.rs`](../../src/providers/registry/mod.rs) |
| **Config** | [`config/providers/nuget.toml`](../../config/providers/nuget.toml) (no tunables) |

## Why
NuGet is the .NET package index. Its search service exposes a keyless JSON API —
the authoritative way to find a NuGet package.

## Features
- Keyless search of the NuGet search service `/query?q=` (with `take` = the
  limit).
- Results live under `/data`, keyed by `id`; each becomes `id version` →
  `https://www.nuget.org/packages/<id>` with the description as the snippet.

## Skills (tools)
- **General:** part of `docs_search` when `nuget` is in
  [`config/02-search.toml`](../../config/02-search.toml) (`[providers].docs`).
- **Per-provider:** `docs_nuget` (args `query`, `max_results?`).

## Schema / structs
Declares a [`RegistrySpec`](../../src/providers/registry/mod.rs):

```rust
RegistrySpec {
    id: "nuget",
    url: "https://azuresearch-usnc.nuget.org/query",
    query_key: "q",
    size_key: Some("take"),
    extra_params: &[],
    results_ptr: "/data",
    item: ItemMap {
        name: "/id",
        description: "/description",
        url_field: None,
        url_template: Some("https://www.nuget.org/packages/{name}"),
        url_base: "",
        version: Some("/version"),
    },
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
