# dockerhub

|  |  |
| --- | --- |
| **Family** | registry (`RegistryProvider` + `RegistrySpec`) |
| **Kind** | `docs` |
| **Default-on** | no — opt-in (add `"dockerhub"` to `[providers].docs`) |
| **Keyless** | yes |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/registry/dockerhub.rs`](../../src/providers/registry/dockerhub.rs) · shared: [`registry/mod.rs`](../../src/providers/registry/mod.rs) |
| **Config** | [`config/providers/dockerhub.toml`](../../config/providers/dockerhub.toml) (no tunables) |

## Why
Docker Hub is the default container image registry. Its keyless JSON search is the
authoritative way to find an image.

## Features
- Keyless search of `hub.docker.com/v2/search/repositories/?query=` (with
  `page_size` = the limit).
- Results under `/results`; each becomes `repo_name` →
  `https://hub.docker.com/r/<repo_name>` with the short description as the snippet.

## Skills (tools)
- **General:** part of `docs_search` when `dockerhub` is in
  [`config/02-search.toml`](../../config/02-search.toml) (`[providers].docs`).
- **Per-provider:** `docs_dockerhub` (args `query`, `max_results?`).

## Schema / structs
Declares a [`RegistrySpec`](../../src/providers/registry/mod.rs):

```rust
RegistrySpec {
    id: "dockerhub",
    url: "https://hub.docker.com/v2/search/repositories/",
    query_key: "query",
    size_key: Some("page_size"),
    extra_params: &[],
    results_ptr: "/results",
    item: ItemMap {
        name: "/repo_name",
        description: "/short_description",
        url_field: None,
        url_template: Some("https://hub.docker.com/r/{name}"),
        url_base: "",
        version: None,
    },
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
