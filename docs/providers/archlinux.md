# archlinux

|  |  |
| --- | --- |
| **Family** | registry (`RegistryProvider` + `RegistrySpec`) |
| **Kind** | `docs` |
| **Default-on** | no — opt-in (add `"archlinux"` to `[providers].docs`) |
| **Keyless** | yes |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/registry/archlinux.rs`](../../src/providers/registry/archlinux.rs) · shared: [`registry/mod.rs`](../../src/providers/registry/mod.rs) |
| **Config** | [`config/providers/archlinux.toml`](../../config/providers/archlinux.toml) (no tunables) |

## Why
Arch Linux's official package database has a keyless JSON search — the
authoritative way to find an official Arch package (complements [`aur`](aur.md)).

## Features
- Keyless search of `archlinux.org/packages/search/json/?q=`.
- The canonical package URL spans `repo`/`arch`/`pkgname`, built via JSON-pointer
  template placeholders → `https://archlinux.org/packages/<repo>/<arch>/<pkgname>/`,
  with the description as the snippet and `pkgver` shown in the title/meta.

## Skills (tools)
- **General:** part of `docs_search` when `archlinux` is in
  [`config/02-search.toml`](../../config/02-search.toml) (`[providers].docs`).
- **Per-provider:** `docs_archlinux` (args `query`, `max_results?`).

## Schema / structs
Declares a [`RegistrySpec`](../../src/providers/registry/mod.rs) whose template
uses `{/pointer}` placeholders for the multi-segment URL:

```rust
RegistrySpec {
    id: "archlinux",
    url: "https://archlinux.org/packages/search/json/",
    query_key: "q",
    size_key: None,
    extra_params: &[],
    results_ptr: "/results",
    item: ItemMap {
        name: "/pkgname",
        description: "/pkgdesc",
        url_field: None,
        url_template: Some("https://archlinux.org/packages/{/repo}/{/arch}/{/pkgname}/"),
        url_base: "",
        version: Some("/pkgver"),
    },
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
