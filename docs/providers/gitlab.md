# gitlab

|  |  |
| --- | --- |
| **Family** | forge (`ForgeCodeProvider` + `ForgeSpec`) |
| **Kind** | `code` |
| **Default-on** | no — opt-in (add `"gitlab"` to `[providers].code`) |
| **Keyless** | yes |
| **Render** | honored per call (passed through to the underlying engine search) |
| **Code** | [`src/providers/forge/gitlab.rs`](../../src/providers/forge/gitlab.rs) · shared: [`forge/mod.rs`](../../src/providers/forge/mod.rs) |
| **Config** | [`config/providers/gitlab.toml`](../../config/providers/gitlab.toml) |

## Why
GitLab hosts a large amount of code that GitHub-centric tools miss. Rather than
use GitLab's API (rate-limited, token-gated for much of it), this is a **keyless,
site-scoped web search** of `gitlab.com` reusing the shared forge machinery
(`forge::search`: DuckDuckGo → Mojeek, render-aware). The only GitLab-specific
knowledge is its blob-URL layout, captured declaratively.

## Features
- Site-scoped code search of `gitlab.com` (scrape-first, render-optional).
- Parses GitLab blob URLs (`/-/blob/<ref>/<path>`) into `(owner/repo, path)` so
  results carry `repo`/`path` and read cleanly.
- A hit's file is fetched with [`fetch_repo_file`](../../src/retrieve.rs), which
  understands GitLab `/-/blob/` → `/-/raw/`.

## Caveats
Result quality depends on the search engines having indexed `gitlab.com` (usually
thinner than GitHub). Not in the default list — add it to opt in.

## Skills (tools)
- **General:** part of `code_search` when `gitlab` is in `[providers].code`.
- **Per-provider:** `code_gitlab` (args `query`, `max_results?`, `language?`,
  `render?`).

## Schema / structs
Declares a [`ForgeSpec`](../../src/providers/forge/mod.rs):

```rust
ForgeSpec {
    id: "gitlab",
    domain: "gitlab.com",
    repo_path: extract,   // regex: /-/blob/<ref>/<path> → (repo, path)
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
