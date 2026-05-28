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
    mod.rs       Provider factory `make()` + shared helpers (scoped queries,
                 result zipping, `finish`, the configurable code-site list).
    engine/      Spec-driven web/code search engines (HtmlEngineProvider +
                 EngineSpec): duckduckgo, mojeek, google.
    forge/       Spec-driven code forges (ForgeCodeProvider + ForgeSpec):
                 github_web, gitlab, codeberg, gitea.
    grep_app.rs                 grep.app JSON code search (bespoke).
    github_api.rs               Authenticated GitHub code search, token (bespoke).
    medium.rs                   Medium tag RSS (bespoke).
    stackexchange.rs            StackExchange API + render-scrape (bespoke).
  browser.rs     PageRenderer trait + a persistent, process-shared
                 ChromiumRenderer. Any provider can render on demand.
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

`browser.rs` exposes a `PageRenderer` trait and a process-wide `ChromiumRenderer`
via `browser::shared_global()` (always compiled in; a Chrome binary is only
needed at runtime when a render path actually runs). The `SearchQuery::render`
flag (set per call by the model, and `fetch_page`'s `render` arg) lets any
HTML-scraping source fetch through the headless browser instead of plain HTTP.
The `engine` family honors it automatically; bespoke providers that scrape HTML
branch on `query.render` and call `crate::browser::shared_global().render(url)`
themselves (see `stackexchange.rs`).

## The provider paradigm

Sources fall into three tiers, from most-shared to most-specific. **Prefer the
highest tier that fits:** push everything generic into shared code and keep only
the genuinely-unique bits in per-source files.

**Tier 1 — the universal interface (`provider.rs`).**
Every source, however implemented, is a `SearchProvider`: `id()`, `kind()`,
`async search(...) -> Vec<SearchResult>`. The `Registry` only ever sees this
trait — it has no idea whether a provider scrapes HTML, calls a JSON API, or
reads RSS. This is what lets providers be combined uniformly (fallback chain or
aggregate meta-search) and selected from config.

**Tier 2 — spec-driven families (a shared provider + a declarative spec).**
When several sources share the SAME logic and differ only in *data*, model the
logic ONCE as a provider parameterized by a small declarative spec, and make each
source a tiny file that just declares its spec:

| Family (dir) | Shared provider | Declarative spec | Members (one file each) |
| --- | --- | --- | --- |
| `engine/` (web search) | `HtmlEngineProvider` | `EngineSpec` — url, `Method` (GET/POST/Browser), `Extract` (two CSS selectors *or* a custom fn), code-scope, extra params | duckduckgo, mojeek, google |
| `forge/` (code forges) | `ForgeCodeProvider` | `ForgeSpec` — id, domain, blob-URL → `(repo, path)` parser | github_web, gitlab, codeberg, gitea |

Google is an engine too — it just declares `Method::Browser` (always render via
headless Chrome) and an `Extract::Custom` parser for its messy markup, instead
of plain GET + two selectors. A future Bing engine would look the same.

Families also **compose**: `ForgeCodeProvider` runs its searches *through* the
`engine` family (DuckDuckGo → Mojeek). Adding a member is a few declarative lines
— no new control flow, no risk to the existing members.

**Tier 3 — bespoke providers (implement the trait directly).**
When a source's transport or parsing is genuinely unique, write a normal
`SearchProvider` in its own file. These don't fit a spec because their wire
formats differ: `grep_app` (JSON code API), `github_api` (authenticated GitHub
API), `medium` (tag RSS/XML), `stackexchange` (keyless API + optional render
scrape). Forcing them into a shared spec would just turn the spec into a bag of
callbacks, so they stay bespoke.

**Decision rule:** is this source the *same shape* as an existing family — an
HTML search engine, or a code forge? If yes, add a spec (tier 2). If its
transport/parsing is unique, add a bespoke provider (tier 3). Either way it
becomes a `SearchProvider` the registry treats identically (tier 1).

### Adding a web engine (tier 2)

1. Create `src/providers/engine/<name>.rs` with `pub(super) static SPEC: EngineSpec`
   — endpoint URL, `Method::Get`/`PostForm`, the two CSS selectors, and a
   `CodeScope` (`SiteOperator` if it supports `site:`, else `Keyword`).
2. Add `mod <name>;` and a `make()` arm in `providers/engine/mod.rs`.
3. Add the id to the `engine` arm in `providers::make()`, a
   `config/providers/<name>.toml`, and the `02-search.toml`/README lists.

### Adding a code forge (tier 2)

1. Create `src/providers/forge/<name>.rs` with `pub(super) static SPEC: ForgeSpec`
   (`id`, `domain`, and a `fn(&str) -> Option<(repo, path)>` blob-URL parser).
2. Add `mod <name>;`, a `make()` arm, and the spec to `SPECS` in
   `providers/forge/mod.rs`.
3. Register the id in `providers::make()` and add `config/providers/<name>.toml`.

### Adding a bespoke provider (tier 3)

1. Create `src/providers/<name>.rs` implementing `SearchProvider`. Do all `.await`
   first to get owned data, then parse **synchronously** (see the invariant
   below). For HTML scraping, honor `query.render` via `browser::shared_global()`.
2. Add `mod <name>;` and a `make()` arm in `providers/mod.rs`; run code results
   through `super::finish(...)` for forge filtering/enrichment.
3. Add `config/providers/<name>.toml` and document the id.

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

## Build & verify

There are no Cargo features — the headless browser (`chromiumoxide`) is always
compiled in; a Chrome/Chromium binary is only needed at runtime when a render or
Google path actually runs.

```sh
cargo build
cargo fmt
cargo clippy --all-targets -- -D warnings
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
