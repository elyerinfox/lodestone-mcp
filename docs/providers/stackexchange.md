# stackoverflow / stackexchange

|  |  |
| --- | --- |
| **Family** | composite (multi-mode Q&A provider) |
| **Kind** | `qa` |
| **Provider id** | `stackoverflow` (the StackExchange network; pick a site with `site`) |
| **Default-on** | yes (qa) |
| **Keyless** | yes (optional API key raises quota) |
| **Render** | honored — `render=true` scrapes (stackoverflow site only) |
| **Code** | [`src/providers/composite/stackexchange.rs`](../../src/providers/composite/stackexchange.rs) · answers: [`src/retrieve.rs`](../../src/retrieve.rs) |
| **Config** | [`config/providers/stackexchange.toml`](../../config/providers/stackexchange.toml) |

## Why
StackExchange is where developers go to **search topics and read answers**, so the
provider exposes both — and both are treated as StackExchange-specific skills (the
generic `qa_search` aggregator simply runs whatever `qa` providers are configured,
currently just this one). It's a **composite** because it chooses between two
sourcing modes per call via the same `render` lever used everywhere:

- **`render=false` (default): keyless API** — `api.stackexchange.com`
  (~300 req/day/IP, shared). An optional key raises that quota (it is *not* a
  login).
- **`render=true`: scrape** — loads `stackoverflow.com/search` in the shared
  headless browser (no quota). The scrape path is **stackoverflow-only**; other
  network sites always use the API.

## Features
- Search any StackExchange-network site via the `site` slug (e.g. `serverfault`,
  `superuser`, `askubuntu`, `unix`).
- **Read answers:** the `qa_stackoverflow_answers` skill returns a question's body
  plus its top answers (by votes, accepted flagged), with code blocks preserved —
  always via the API.
- **Guardrail:** `allowed_sites` is an allowlist of site slugs; a request for any
  other site is rejected. Empty list = allow any.
- Optional API key from `[stackexchange].key` or `LODESTONE_STACKEXCHANGE_KEY`
  (never logged or committed).

## Skills (tools)
- **General:** part of `qa_search` (the kind-level aggregator).
- **Per-provider:** `qa_stackoverflow` — search this source directly (args
  `query`, `site?`, `max_results?`, `render?`).
- **Per-provider (bespoke):** `qa_stackoverflow_answers` — read a question's body
  and top answers (args `question` [URL or numeric id], `site?`, `max_answers?`).

## Schema / structs
The provider struct and its mode switch
([`composite/stackexchange.rs`](../../src/providers/composite/stackexchange.rs)):

```rust
pub(crate) struct StackExchange { key: String }

// SearchProvider::search:
//   render && site == "stackoverflow" → search_scrape()  // headless browser
//   else                              → search_api(site)  // api.stackexchange.com/2.3
```

Config:

```toml
[stackexchange]
default_site  = "stackoverflow"   # used when a call omits `site`
key           = ""                # optional; or env LODESTONE_STACKEXCHANGE_KEY
allowed_sites = []                # allowlist of site slugs; empty = any
```

Site slugs are the `api_site_parameter` values from
<https://api.stackexchange.com/2.3/sites> (slugs, **not** URLs).

Shared types: `SearchProvider`, `SearchQuery` (uses `site`), `SearchResult` (uses
`score`/`meta`) ([`src/provider.rs`](../../src/provider.rs)). Per-provider search
args: `ProviderSearchArgs`; answers args: `StackAnswersArgs`
([`src/main.rs`](../../src/main.rs)).
