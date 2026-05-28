# aur

|  |  |
| --- | --- |
| **Family** | registry (`RegistryProvider` + `RegistrySpec`) |
| **Kind** | `docs` |
| **Default-on** | no — opt-in (add `"aur"` to `[providers].docs`) |
| **Keyless** | yes |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/registry/aur.rs`](../../src/providers/registry/aur.rs) · shared: [`registry/mod.rs`](../../src/providers/registry/mod.rs) |
| **Config** | [`config/providers/aur.toml`](../../config/providers/aur.toml) (no tunables) |

## Why
The Arch User Repository hosts community-maintained Arch packages, with a keyless
JSON RPC search — the authoritative way to find an AUR package.

## Features
- Keyless search of the AUR RPC (`/rpc/?v=5&type=search&arg=`).
- Results under `/results`; each becomes `Name Version` →
  `https://aur.archlinux.org/packages/<Name>` with the description as the snippet.

## Skills (tools)
- **General:** part of `docs_search` when `aur` is in
  [`config/02-search.toml`](../../config/02-search.toml) (`[providers].docs`).
- **Per-provider:** `docs_aur` (args `query`, `max_results?`).

## Schema / structs
Declares a [`RegistrySpec`](../../src/providers/registry/mod.rs):

```rust
RegistrySpec {
    id: "aur",
    url: "https://aur.archlinux.org/rpc/",
    query_key: "arg",
    size_key: None,
    extra_params: &[("v", "5"), ("type", "search")],
    results_ptr: "/results",
    item: ItemMap {
        name: "/Name",
        description: "/Description",
        url_field: None,
        url_template: Some("https://aur.archlinux.org/packages/{name}"),
        url_base: "",
        version: Some("/Version"),
    },
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
