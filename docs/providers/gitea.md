# gitea

|  |  |
| --- | --- |
| **Family** | forge (`ForgeCodeProvider` + `ForgeSpec`) |
| **Kind** | `code` |
| **Default-on** | no — opt-in (add `"gitea"` to `[providers].code`) |
| **Keyless** | yes |
| **Render** | honored per call (passed through to the underlying engine search) |
| **Code** | [`src/providers/forge/gitea.rs`](../../src/providers/forge/gitea.rs) · shared: [`forge/mod.rs`](../../src/providers/forge/mod.rs) |
| **Config** | [`config/providers/gitea.toml`](../../config/providers/gitea.toml) |

## Why
Covers the public `gitea.com` instance — the reference deployment of the Gitea
forge. Same rationale as the other forges: a **keyless, site-scoped web search**
over the shared `forge::search` machinery (DuckDuckGo → Mojeek, render-aware),
with only the Gitea blob-URL layout declared per-provider. ([`codeberg`](codeberg.md)
is the same family pointed at `codeberg.org`.)

## Features
- Site-scoped code search of `gitea.com` (scrape-first, render-optional).
- Parses Gitea blob URLs (`/src/branch|commit|tag/<ref>/<path>`) into
  `(owner/repo, path)`.
- A hit's file is fetched with [`fetch_repo_file`](../../src/retrieve.rs)
  (Gitea `/src/...` → `/raw/...`).

## Caveats
Depends on engines indexing `gitea.com` (relatively small). Not in the default
list — add it to opt in.

## Skills (tools)
- **General:** part of `code_search` when `gitea` is in `[providers].code`.
- **Per-provider:** `code_gitea` (args `query`, `max_results?`, `language?`,
  `render?`).

## Schema / structs
Declares a [`ForgeSpec`](../../src/providers/forge/mod.rs):

```rust
ForgeSpec {
    id: "gitea",
    domain: "gitea.com",
    repo_path: extract,   // regex: /src/(branch|commit|tag)/<ref>/<path> → (repo, path)
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
