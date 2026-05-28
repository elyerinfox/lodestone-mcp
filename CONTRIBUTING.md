# Contributing to lodestone-mcp

This guide explains how the codebase is laid out, the few invariants that keep it
correct, and how to extend it (the common case: adding a search provider).

## Golden rules (non-negotiable)

These are project invariants. New code and providers must uphold them; a change
that breaks one is wrong by definition.

1. **Scrape is the default; render is optional and a fallback.** Every source
   fetches over plain HTTP by default. The headless browser is never the default
   path — it runs only when the model explicitly asks for it (a `render` flag on
   a search, or the dedicated `render_page` tool), as its fallback when a plain
   fetch isn't enough. The server never silently substitutes rendering. (The sole
   exception is the `google` engine, which has no scrapeable endpoint and is
   therefore browser-only and strictly opt-in via config.)
2. **The LLM always decides.** Rendering is a per-call `render` flag the calling
   model sets; the server never enables it on its own. The model likewise drives
   what to retrieve next. We expose capabilities and defaults — we don't make the
   call for it.
3. **Keyless by default.** No source requires an account or key on the default
   path. Credentials (a GitHub token, a StackExchange key) are strictly optional
   enhancements layered over a keyless fallback, never a precondition.
4. **Parallelize — always.** Independent work must run concurrently, never
   sequentially. Aggregate search sources every provider on its own task across
   the multi-threaded runtime; any new multi-source or I/O-bound path must
   overlap its work (`tokio::spawn` / `join`) and must never block the runtime
   with sync I/O or long CPU work on the async threads.

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
    forge/       Spec-driven code forges (ForgeCodeProvider + ForgeSpec, and the
                 shared `forge::search`): gitlab, codeberg, gitea.
    github.rs                   Composite: forge scrape (default) + GitHub API (token).
    grep_app.rs                 grep.app JSON code search (bespoke).
    medium.rs                   Medium tag RSS (bespoke).
    stackexchange.rs            StackExchange API + render-scrape (composite).
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
flag (set per call by the model on the search tools) and the dedicated
`render_page` tool let any HTML-scraping path fetch through the headless browser
instead of plain HTTP. The `engine` family honors the flag automatically;
bespoke providers that scrape HTML branch on `query.render` and call
`crate::browser::shared_global().render(url)` themselves (see `stackexchange.rs`).

## The provider paradigm

> For a detailed, per-provider reference (what each one does, keyless vs.
> credentialed, config, caveats), see [docs/providers.md](docs/providers.md).
> This section is about the *architecture*; that page is about the *providers*.

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
| `forge/` (code forges) | `ForgeCodeProvider` / `forge::search` | `ForgeSpec` — id, domain, blob-URL → `(repo, path)` parser | gitlab, codeberg, gitea (GitHub reuses `forge::search` — see below) |

Google is an engine too — it just declares `Method::Browser` (always render via
headless Chrome) and an `Extract::Custom` parser for its messy markup, instead
of plain GET + two selectors. A future Bing engine would look the same.

Families also **compose**: `ForgeCodeProvider` runs its searches *through* the
`engine` family (DuckDuckGo → Mojeek). Adding a member is a few declarative lines
— no new control flow, no risk to the existing members.

**Tier 3 — bespoke providers (implement the trait directly).**
When a source's transport or parsing is genuinely unique, write a normal
`SearchProvider` in its own file. These don't fit a spec because their wire
formats differ: `grep_app` (JSON code API), `medium` (tag RSS/XML). Forcing them
into a shared spec would just turn the spec into a bag of callbacks, so they stay
bespoke.

**Composite providers.** Some sources have more than one mode and pick one at
runtime — these are bespoke shells that *dispatch* (and often reuse a family for
one mode), honoring the golden rules:

- `github` — **scrape by default** (reuses `forge::search` with a github
  `ForgeSpec`); switches to the authenticated GitHub **API** only when a token is
  set. GitHub's keyless half is a forge; its API half isn't, so the whole thing
  is composite rather than a plain forge member.
- `stackexchange` — keyless **API** by default; scrapes via the headless browser
  only when the caller sets `render=true`.

**Decision rule:** is this source the *same shape* as an existing family — an
HTML search engine, or a code forge? If yes, add a spec (tier 2). If its
transport/parsing is unique, add a bespoke provider (tier 3). Either way it
becomes a `SearchProvider` the registry treats identically (tier 1).

### Adding a web engine (tier 2)

1. Create `src/providers/engine/<name>.rs` with `pub(super) static SPEC: EngineSpec`
   — endpoint URL, a `Method` (`Get`/`PostForm`, or `Browser` to always render),
   an `Extract` (two CSS selectors, or a `Custom` parser fn for messy markup), a
   `CodeScope` (`SiteOperator` if it supports `site:`, else `Keyword`), and any
   fixed `extra_params`.
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
