# google

|  |  |
| --- | --- |
| **Family** | engine (`HtmlEngineProvider` + `EngineSpec`) |
| **Kinds** | `web`, `code` |
| **Default-on** | no — opt-in (add `"google"` to the lists) |
| **Keyless** | yes (no API key) |
| **Render** | **always** (`Method::Browser`) — needs a local Chrome |
| **Code** | [`src/providers/engine/google.rs`](../../src/providers/engine/google.rs) · shared: [`engine/mod.rs`](../../src/providers/engine/mod.rs) |
| **Config** | [`config/providers/google.toml`](../../config/providers/google.toml) |

## Why
Google has the broadest index but no scrapeable plain-HTTP surface and no keyless
API, so it is the project's **one always-render provider**: every query loads
`google.com/search` in the shared headless browser so the request looks like a
real browser. This is the deliberate exception to the *scrape-is-default* golden
rule — documented and opt-in rather than silent. Its results markup needs real
logic, so it uses a custom parser instead of plain selectors.

## Features
- **Web + code search** (code via `site:`, `CodeScope::SiteOperator`).
- Custom parser walks each `<h3>` up to its result anchor and normalizes the
  href (handles the `/url?q=` redirector, drops internal Google links).
- Detects CAPTCHA / consent interstitials and logs a warning when blocked.

## Caveats
- Requires a local **Chrome/Chromium** at runtime (the always-render path).
- CAPTCHA-prone on datacenter IPs and subject to a regional consent page — keep a
  tolerant engine ([`mojeek`](mojeek.md)) in the chain as a fallback.

## Skills (tools)
- **General:** part of `web_search` / `code_search` only when `google` is added
  to a list in [`config/02-search.toml`](../../config/02-search.toml).
- **Per-provider:** `web_google`, `code_google` (args `query`, `max_results?`,
  `language?`; `render` is implied — it always renders).

## Schema / structs
Declares an [`EngineSpec`](../../src/providers/engine/mod.rs) with a `Browser`
method and a `Custom` extractor:

```rust
EngineSpec {
    id: "google",
    url: "https://www.google.com/search",
    method: Method::Browser,           // always headless
    extract: Extract::Custom(parse),   // bespoke parser
    code_scope: CodeScope::SiteOperator,
    extra_params: &[("hl", "en"), ("gl", "us"), ("num", "20")],
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)). Per-provider tool args:
`ProviderSearchArgs` ([`src/main.rs`](../../src/main.rs)).
