# github

|  |  |
| --- | --- |
| **Family** | composite (bespoke shell that reuses the forge machinery) |
| **Kind** | `code` |
| **Default-on** | no — opt-in (add `"github"` to `[providers].code`) |
| **Keyless** | yes (optional token raises capability) |
| **Render** | honored on the keyless scrape path (ignored by the API path) |
| **Code** | [`src/providers/composite/github.rs`](../../src/providers/composite/github.rs) |
| **Config** | [`config/providers/github.toml`](../../config/providers/github.toml) → `[github].token` |

## Why
GitHub is the largest code host, but it **dropped unauthenticated API code
search**, so it can't be a plain forge provider. It's a **composite**: one
provider that picks its sourcing mode at runtime —

- **default (keyless): scrape** — a site-scoped web search of `github.com`,
  reusing the shared `forge::search` (DuckDuckGo → Mojeek, render-aware). Keeps
  the *keyless-by-default* golden rule intact.
- **token set: API** — GitHub's authenticated code-search API
  (`api.github.com/search/code`) with the text-match media type, returning matched
  code fragments as snippets. A strict opt-in *enhancement*, never required.

This is why GitHub lives in `composite/` rather than `forge/`: the forge family is
purely keyless web search, while GitHub adds a second, credentialed path. (Its URL
layout is still understood by the forge `repo_path` resolver and by
`fetch_repo_file`.)

## Features
- Keyless GitHub code search with zero setup (scrape path).
- Optional token unlocks the real code-search API (more precise, true code
  fragments) — read from `[github].token`, `GITHUB_TOKEN`, or
  `LODESTONE_GITHUB_TOKEN`. The token is never logged or committed.
- Render applies only to the keyless scrape path; the API path is HTTP-JSON.

## Skills (tools)
- **General:** part of `code_search` when `github` is in `[providers].code`.
- **Per-provider:** `code_github` (args `query`, `max_results?`, `language?`,
  `render?`). `language` maps to the API `language:` qualifier when a token is set.
- **Retrieve:** open any result with [`fetch_repo_file`](../../src/retrieve.rs)
  (GitHub `/blob/` → `raw.githubusercontent.com`, or `owner/repo/path` shorthand).

## Schema / structs
The provider struct and its two modes
([`composite/github.rs`](../../src/providers/composite/github.rs)):

```rust
pub(crate) struct Github { token: String }

// keyless path reuses a ForgeSpec:
static SPEC: ForgeSpec = ForgeSpec { id: "github", domain: "github.com", repo_path: extract };

// SearchProvider::search: token empty → forge::search(&SPEC, …)
//                         token set   → self.search_api(…)  // api.github.com/search/code
```

Config:

```toml
[github]
token = ""   # optional fine-grained PAT; or env GITHUB_TOKEN / LODESTONE_GITHUB_TOKEN
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
