# Contributing to lodestone-mcp

This guide explains how the codebase is laid out, the few invariants that keep it
correct, and how to extend it (the common case: adding a search provider).

## Architecture at a glance

```
src/
  main.rs        Bootstrap + the MCP tools. Loads config, builds the Registry,
                 configures the renderer and forge sites, serves Streamable-HTTP
                 at /mcp. Defines the #[tool] methods and output formatting.
  provider.rs    The core interface: SearchProvider trait, ProviderKind,
                 Strategy, SearchQuery, SearchResult, and the Registry that
                 combines providers (fallback chain or aggregate meta-search).
  providers/
    mod.rs       Provider factory `make()` + shared helpers (fetch_html,
                 site/keyword scoped queries, result zipping, `finish`,
                 forge URL parsing, the configurable code-site list).
    duckduckgo.rs / mojeek.rs   HTML-scraping web+code engines.
    grep_app.rs                 grep.app JSON code search.
    github_api.rs               Authenticated GitHub code search (token).
    stackexchange.rs            Keyless StackExchange API (Q&A).
    google.rs                   Headless-Chrome Google (feature `google`).
    stackoverflow_scrape.rs     SO via headless browser (feature `browser`).
  browser.rs     (feature `browser`) PageRenderer trait + a persistent,
                 process-shared ChromiumRenderer. Any provider can render.
  retrieve.rs    Retrieval of one known resource: raw GitHub files, readable
                 page text, Wayback snapshots, StackExchange answer threads.
  config.rs      Config struct + file (TOML) and env-var loading.
  util.rs        HTML→text, whitespace/entity helpers, truncation.
```

## Core concepts

### Providers vs. retrieval

- A **provider** (`providers/`) ranks many candidates for a query and implements
  `SearchProvider`. Providers are pluggable and config-selected.
- **Retrieval** (`retrieve.rs`) fetches one specific, already-identified thing
  (a file, a page, a Q&A thread). These are plain functions, not providers.

### The provider interface

```rust
#[async_trait]
pub trait SearchProvider: Send + Sync {
    fn id(&self) -> &'static str;                // config id / attribution
    fn kind(&self) -> ProviderKind;              // Web | Code | Qa
    async fn search(&self, http: &Client, query: &SearchQuery)
        -> anyhow::Result<Vec<SearchResult>>;
}
```

- Return an **empty vec** for "no results" (the registry moves on).
- Return **`Err`** for transport/parse failures (logged as a warning, skipped).
- The `Registry` runs providers per `ProviderKind` using the configured
  `Strategy`: `Fallback` (first non-empty wins) or `Aggregate` (run all, dedupe
  by URL, re-rank, annotate engines).

### Rendering is shared and model-controlled

`browser.rs` exposes a `PageRenderer` trait and a process-wide
`ChromiumRenderer` via `browser::shared_global()`. Any provider can render a URL
on demand. The `SearchQuery::render` flag (set per call by the model on
`web_search`/`code_search`, and `fetch_page`'s `render` arg) decides whether the
shared helper `providers::fetch_html` uses the browser or plain HTTP. Keep the
browser path behind `#[cfg(feature = "browser")]`.

## Adding a search provider

1. Create `src/providers/<name>.rs`:

   ```rust
   use anyhow::Result;
   use async_trait::async_trait;
   use reqwest::Client;

   use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};

   pub(super) struct MyEngine;

   #[async_trait]
   impl SearchProvider for MyEngine {
       fn id(&self) -> &'static str { "myengine" }
       fn kind(&self) -> ProviderKind { ProviderKind::Web }
       async fn search(&self, http: &Client, query: &SearchQuery)
           -> Result<Vec<SearchResult>> {
           // HTML scraper? Use the shared fetcher so `render` works:
           let url = format!("https://example.com/search?q={}", query.text);
           let body = super::fetch_html(http, query, &url).await?;
           Ok(parse(&body, query.limit)) // sync parse, owned output
       }
   }

   fn parse(body: &str, max: usize) -> Vec<SearchResult> { /* scraper here */ }
   ```

2. Register it in `providers/mod.rs`: add `mod <name>;` and a match arm in
   `make()` (gate with `#[cfg(feature = "...")]` if it needs the browser).
3. Add a `config/providers/<id>.toml` file (settings, or documentation only)
   and document the id in `config/02-search.toml` and the README provider table.
4. Code providers should run results through `super::finish(...)` so forge
   filtering and repo/path enrichment apply.

## Invariants & conventions

- **Never hold a `scraper` value across `.await`.** `Html`, `Selector`, and
  `ElementRef` are `!Send`; the tool futures must be `Send`. Do all awaits first
  to obtain an owned `String`, then parse in a **synchronous** function that
  returns owned data. (This is why every provider has a `fn parse(...)`.)
- **Errors:** providers/retrieval return `anyhow::Result`; the tool layer in
  `main.rs` maps them with `internal()` / `invalid()` to MCP errors.
- **Keyless ethos:** don't introduce a source that requires a key/account unless
  it's optional, documented, and has a keyless fallback. Never log secrets.
- **No secrets in git:** `lodestone.toml` is gitignored; commit changes to
  the committed `config/` baseline / `examples/` instead. Prefer `GITHUB_TOKEN`
  via env over any file.
- Keep comments about *why*, not *what*; let names carry the rest.

## Feature flags

- `browser` — pulls in `chromiumoxide`, the `PageRenderer`/`ChromiumRenderer`,
  the `render` path in `fetch_html`/`fetch_page`, and the `stackoverflow_scrape`
  provider. Requires a local Chrome/Chromium at runtime.
- `google` — implies `browser`; adds the Google provider.

Build/verify both so neither path rots:

```sh
cargo build
cargo build --features google
cargo fmt
cargo clippy --all-features
```

## Adding a tool

Add an `async fn` to the `#[tool_router] impl Lodestone` block in `main.rs` with
a `#[tool(description = "…")]` attribute and a `Parameters<Args>` argument whose
`Args` struct derives `Deserialize` + `schemars::JsonSchema`. Return
`CallToolResult` (use the `text_result(...)` helper). Mention it in `get_info`.

## Manual smoke test

Run the server, then drive the MCP handshake over HTTP (initialize → capture
`Mcp-Session-Id` → `notifications/initialized` → `tools/list` / `tools/call`).
The streamable endpoint returns server-sent events; the JSON-RPC response is in
the `data:` line.
