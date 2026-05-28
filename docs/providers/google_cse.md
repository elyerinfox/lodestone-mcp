# google_cse

|  |  |
| --- | --- |
| **Family** | apiengine (`ApiProvider` + `ApiSpec`) |
| **Kind** | `web` |
| **Default-on** | no — **keyed**, opt-in (set key + cx, add `"google_cse"` to `[providers].web`) |
| **Keyless** | no — requires an API key + search-engine id (optional; never required) |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/apiengine/google.rs`](../../src/providers/apiengine/google.rs) · shared: [`apiengine/mod.rs`](../../src/providers/apiengine/mod.rs) |
| **Config** | [`config/providers/google_cse.toml`](../../config/providers/google_cse.toml) → `[google_cse].key` + `.cx` |

## Why
Google's official Programmable Search (Custom Search JSON API) gives Google-quality
results without scraping, for users who have a key. Distinct from the keyless
`google` engine (which drives headless Chrome). **Optional** per golden rule 3.

## Features
- GET `googleapis.com/customsearch/v1?q=` with `key` and `cx` query params;
  results parsed from `/items` (title/link/snippet). `num` is capped at the API
  max (10).

## Enabling
1. Create an API key (Google Cloud, Custom Search API enabled) and a Programmable
   Search Engine id (`cx`) at <https://programmablesearchengine.google.com/>
   (configured to search the whole web).
2. Set `LODESTONE_GOOGLE_CSE_KEY` + `LODESTONE_GOOGLE_CSE_CX` (preferred), or
   `[google_cse].key` + `.cx`.
3. Add `"google_cse"` to `[providers].web` in `config/02-search.toml`.

## Skills (tools)
- **General:** part of `web_search` when `google_cse` is configured + enabled.
- **Per-provider:** `web_google_cse` (args `query`, `max_results?`).

## Schema / structs
Declares an [`ApiSpec`](../../src/providers/apiengine/mod.rs); `cx` is supplied as
a credential query param at construction:

```rust
ApiSpec {
    id: "google_cse",
    url: "https://www.googleapis.com/customsearch/v1",
    query_key: "q",
    size_key: Some("num"),
    size_cap: 10,
    auth: Auth::Query("key"),
    extra_params: &[],
    results_ptr: "/items",
    title: "/title",
    link: "/link",
    snippet: "/snippet",
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
