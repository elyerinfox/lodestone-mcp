# brave

|  |  |
| --- | --- |
| **Family** | apiengine (`ApiProvider` + `ApiSpec`) |
| **Kind** | `web` |
| **Default-on** | no — **keyed**, opt-in (set `[brave].key`, add `"brave"` to `[providers].web`) |
| **Keyless** | no — requires an API key (optional enhancement; never required) |
| **Render** | n/a (JSON API; the `render` flag is ignored) |
| **Code** | [`src/providers/apiengine/brave.rs`](../../src/providers/apiengine/brave.rs) · shared: [`apiengine/mod.rs`](../../src/providers/apiengine/mod.rs) |
| **Config** | [`config/providers/brave.toml`](../../config/providers/brave.toml) → `[brave].key` |

## Why
Brave runs its own independent index with a clean official JSON API — a strong web
source for users who have a key. It's **optional** (golden rule 3): off unless a
key is set, and it never replaces the keyless providers.

## Features
- GET `api.search.brave.com/res/v1/web/search?q=` with the subscription token in
  the `X-Subscription-Token` header; results parsed from `/web/results`
  (title/url/description). `count` is capped at the API max (20).

## Enabling
1. Get a key at <https://brave.com/search/api/>.
2. Set `LODESTONE_BRAVE_KEY` (preferred) or `[brave].key`.
3. Add `"brave"` to `[providers].web` in `config/02-search.toml`.

## Skills (tools)
- **General:** part of `web_search` when `brave` is configured + enabled.
- **Per-provider:** `web_brave` (args `query`, `max_results?`).

## Schema / structs
Declares an [`ApiSpec`](../../src/providers/apiengine/mod.rs):

```rust
ApiSpec {
    id: "brave",
    url: "https://api.search.brave.com/res/v1/web/search",
    query_key: "q",
    size_key: Some("count"),
    size_cap: 20,
    auth: Auth::Header("X-Subscription-Token"),
    extra_params: &[],
    results_ptr: "/web/results",
    title: "/title",
    link: "/url",
    snippet: "/description",
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
