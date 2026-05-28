# Contributing to lodestone-mcp

This guide explains how the codebase is laid out, the few invariants that keep it
correct, and how to extend it (the common case: adding a search provider).

## Golden rules (non-negotiable)

The project invariants are maintained in one place:
**[docs/golden-rules.md](docs/golden-rules.md)**. New code and providers must
uphold all of them; a change that breaks one is wrong by definition. In brief:

1. Scrape is the default; render is optional (model-controlled).
2. The LLM always decides.
3. Keyless by default (credentials strictly optional).
4. Parallelize — always.
5. Everything is enable/disable-able.
6. Every provider is documented.
7. Every tool is a self-contained skill module under a common contract (no tool
   logic in `main.rs`).

Read [docs/golden-rules.md](docs/golden-rules.md) for the full statement of each.

## Architecture at a glance

```
src/
  main.rs        Bootstrap + wiring ONLY (golden rule 7). Loads config, builds the
                 Registry + shared state (Lodestone), configures the renderer and
                 forge sites, assembles the router from skills, serves
                 Streamable-HTTP at /mcp. No tool logic lives here.
  skills/         Every tool, one module per skill family, implementing the
                 `Skill` contract (name/description/schema/call); mod.rs assembles
                 them into routes and computes config gating (disabled_by_config).
                 A skill owns its domain logic + arg structs + formatters:
                 search, retrieve, archive, rfc, standards, arxiv, huggingface,
                 wikipedia, kernel, github, oci (Docker Hub + OCI), artifacthub,
                 docker (daemon), kubernetes, filesystem, shell, datetime,
                 translate, data (JSON/YAML), regex, math, units, meta.
  provider.rs    The core interface: SearchProvider trait, ProviderKind,
                 Strategy, SearchQuery, SearchResult, and the Registry that
                 combines providers (fallback chain or aggregate meta-search).
  providers/      Providers, grouped by family (one subfolder per family).
    mod.rs       Provider factory `make()` + shared helpers (scoped queries,
                 result zipping, `finish`, the configurable code-site list).
    engine/      Spec-driven search engines (HtmlEngineProvider + EngineSpec):
                 duckduckgo, mojeek, google.
    apiengine/   Spec-driven KEYED JSON web-search APIs (ApiProvider + ApiSpec),
                 off unless a key is set: brave, google_cse.
    forge/       Spec-driven code forges (ForgeCodeProvider + ForgeSpec, and the
                 shared `forge::search`): gitlab, codeberg, gitea.
    registry/    Spec-driven doc/package registries (RegistryProvider +
                 RegistrySpec, JSON APIs, `docs` kind): cratesio, npm, mdn.
    composite/   Multi-mode providers that dispatch (and reuse a family):
                 github (scrape↔API), stackexchange (API↔render).
    bespoke/     Unique transport/parse providers: grep_app (JSON), medium (RSS),
                 searxng (self-hosted metasearch JSON, web+code).
  browser.rs     PageRenderer trait + a persistent, process-shared
                 ChromiumRenderer. Any provider can render on demand.
  config.rs      Config struct + file (TOML) and env-var loading.
  cache.rs       In-memory TTL cache used by the Registry for search results.
  hive/          Opt-in P2P hivemind: Bloom-filter digests, consult-then-fetch
                 with consensus/reputation anti-poisoning, mDNS + gossip
                 discovery, bounded relay across the mesh graph.
  util.rs        HTML→text, whitespace/entity helpers, truncation.
```

## Core concepts

### Terminology: provider vs. skill vs. tool

These three words are used precisely throughout the codebase:

- **Tool** — the MCP wire primitive the model invokes: a `name`, a JSON argument
  schema, and a handler. "Tool" is the protocol-level concept (what shows up in
  `tools/list`).
- **Skill** — *our* abstraction that **implements** a tool: a self-contained module
  under [`src/skills/`](src/skills/) implementing the [`Skill`](src/skills/mod.rs)
  contract (`name` / `description` / `schema` / `call`) and owning its domain logic.
  Every skill produces exactly one tool. This is the unit you add when you add a
  capability (golden rule 7) — never inline in `main.rs`.
- **Provider** — a *data source* implementing
  [`SearchProvider`](src/provider.rs) under [`src/providers/`](src/providers/)
  (kinds: web/code/qa/docs), selected per kind and combined by a strategy. A
  provider is **not** itself a tool; it's surfaced through the search skills
  (`web_search`, …) and an auto-generated per-provider tool `<kind>_<id>`
  (e.g. `code_github`). Skills may build on providers; many skills (translate,
  docker, kubernetes, …) have no provider at all.

So: *providers* are sources of ranked results; *skills* are the modular
capabilities that become *tools*. Search skills consume providers; non-search
skills (e.g. the Docker/Kubernetes/translate families) talk to their own clients.

### Providers vs. retrieval

- A **provider** (`providers/`) ranks many candidates for a query and implements
  `SearchProvider`. Providers are pluggable and config-selected.
- **Retrieval** fetches one specific, already-identified thing (a file, a page, a
  Q&A thread). It is not a provider — the logic lives *with its skill* under
  `skills/` (e.g. the raw-file/page/PDF/Wayback primitives in
  [`skills/retrieve.rs`](src/skills/retrieve.rs); GitHub helpers in
  [`skills/github.rs`](src/skills/github.rs)).

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
  by URL, re-rank, annotate engines). The re-ranking methods (default
  `composite`) are documented in [docs/ranking.md](docs/ranking.md).

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
| `registry/` (doc/package registries, `docs` kind) | `RegistryProvider` | `RegistrySpec` — url, query/size params, results JSON pointer, item map (name/description/url field-or-template/version pointers) | cratesio, npm, mdn, … |
| `apiengine/` (keyed web search, `web` kind) | `ApiProvider` | `ApiSpec` — url, query/size params, `Auth` (key as header or query param), results pointer, title/link/snippet pointers | brave, google_cse (off unless keyed) |

Google is an engine too — it just declares `Method::Browser` (always render via
headless Chrome) and an `Extract::Custom` parser for its messy markup, instead
of plain GET + two selectors. A future Bing engine would look the same.

Families also **compose**: `ForgeCodeProvider` runs its searches *through* the
`engine` family (DuckDuckGo → Mojeek). Adding a member is a few declarative lines
— no new control flow, no risk to the existing members.

**Tier 3 — bespoke providers (implement the trait directly).**
When a source's transport or parsing is genuinely unique, write a normal
`SearchProvider` in its own file. These don't fit a spec because their wire
formats differ: `grep_app` (JSON code API), `medium` (tag RSS/XML), `searxng`
(self-hosted metasearch JSON, serving web+code). Forcing them into a shared spec
would just turn the spec into a bag of callbacks, so they stay bespoke.

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

1. Create `src/providers/bespoke/<name>.rs` implementing `SearchProvider`. Do all
   `.await` first to get owned data, then parse **synchronously** (see the
   invariant below). For HTML scraping, honor `query.render` via
   `browser::shared_global()`.
2. In `bespoke/mod.rs` add `mod <name>;` and `pub(crate) use <name>::<Type>;`,
   then add a `make()` arm in `providers/mod.rs`; run code results through
   `super::finish(...)` for forge filtering/enrichment.
3. Add `config/providers/<name>.toml` and document the id.

A **composite** provider (one that dispatches between modes, like `github` or
`stackexchange`) goes in `src/providers/composite/` the same way — and may reuse
a family (e.g. `crate::providers::forge::search`) for one of its modes.

## Provider contribution checklist

A provider isn't done when it compiles — it's done when an **end user can clone,
run, and understand it without reading the source**. Every new provider PR must
tick all of these:

- [ ] **Resolves by id.** Registered in `providers::make()` (and the family
      `make()`/`SPECS` if spec-driven) so `<id>` works in `config/02-search.toml`.
- [ ] **Config file** `config/providers/<id>.toml` that:
      - documents **every** property it offers — purpose, accepted values/format
        (with examples), default, and the matching `LODESTONE_*` env var; and
      - ships **sane keyless defaults that work out of the box** (or, if the
        provider has no tunables, a short doc-only file saying what it does and
        how to enable it).
- [ ] **Listed in `config/02-search.toml`** under the known ids for its kind.
      Add it to a default `[providers]` list only if it's keyless and reliable
      with zero setup; otherwise document it as opt-in.
- [ ] **Per-provider doc page** `docs/providers/<id>.md` (copy an existing one as
      a template): the header table (family, kind(s), default-on, keyless, render,
      code link, config link), **Why**, **Features**, any **Caveats**, **Skills
      (tools)** (the general tool it joins + its `<kind>_<id>` tool), and
      **Schema / structs** (the spec/struct literal and config keys).
- [ ] **Index row in [docs/providers.md](docs/providers.md)** under its family,
      linking to the new page.
- [ ] **Reference docs updated** — [docs/tools.md](docs/tools.md) (its `<kind>_<id>`
      tool / any bespoke skill) and, for a new family, a row/section in the relevant
      reference. The README is a concise overview; it links to these, so it usually
      needs no per-provider edit.
- [ ] **All [golden rules](docs/golden-rules.md) upheld** — in particular keyless
      by default, scrape-default / render-optional, enable/disable-able, and
      documented.
- [ ] **Stable, snake_case `id`** — it becomes the auto-generated per-provider
      tool name `<kind>_<id>` (e.g. `code_<id>`), so pick it deliberately.
- [ ] **A fixture-based parse test** where practical (pin the scraper/parser).
- [ ] **Credentials, if any:** read from config *and* a `LODESTONE_*` env var,
      never logged, never committed (the live `lodestone.toml` is gitignored).

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
