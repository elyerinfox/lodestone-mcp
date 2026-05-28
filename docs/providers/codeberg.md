# codeberg

|  |  |
| --- | --- |
| **Family** | forge (`ForgeCodeProvider` + `ForgeSpec`) |
| **Kind** | `code` |
| **Default-on** | no — opt-in (add `"codeberg"` to `[providers].code`) |
| **Keyless** | yes |
| **Render** | honored per call (passed through to the underlying engine search) |
| **Code** | [`src/providers/forge/codeberg.rs`](../../src/providers/forge/codeberg.rs) · shared: [`forge/mod.rs`](../../src/providers/forge/mod.rs) |
| **Config** | [`config/providers/codeberg.toml`](../../config/providers/codeberg.toml) |

## Why
Codeberg is a popular community Gitea instance and home to a lot of FOSS code. As
with the other forges, this is a **keyless, site-scoped web search** of
`codeberg.org` over the shared forge machinery (`forge::search`: DuckDuckGo →
Mojeek, render-aware); only its Gitea blob-URL layout is provider-specific.

## Features
- Site-scoped code search of `codeberg.org` (scrape-first, render-optional).
- Parses Gitea blob URLs (`/src/branch|commit|tag/<ref>/<path>`) into
  `(owner/repo, path)`.
- A hit's file is fetched with [`fetch_repo_file`](../../src/retrieve.rs), which
  maps Gitea `/src/...` → `/raw/...`.

## Caveats
Depends on engines indexing `codeberg.org`. Not in the default list — add it to
opt in. (Self-hosted Gitea instances on other domains aren't covered by this
spec; see [`gitea`](gitea.md) for the canonical `gitea.com` variant.)

## Skills (tools)
- **General:** part of `code_search` when `codeberg` is in `[providers].code`.
- **Per-provider:** `code_codeberg` (args `query`, `max_results?`, `language?`,
  `render?`).

## Schema / structs
Declares a [`ForgeSpec`](../../src/providers/forge/mod.rs):

```rust
ForgeSpec {
    id: "codeberg",
    domain: "codeberg.org",
    repo_path: extract,   // regex: /src/(branch|commit|tag)/<ref>/<path> → (repo, path)
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
