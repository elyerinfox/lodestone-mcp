# lodestone-mcp

An [MCP](https://modelcontextprotocol.io) server that gives a local LLM the
ability to **search the web and retrieve code & documentation** — by scraping
search engines and public endpoints rather than calling paid, key-gated APIs.

Built for local runners like **LM Studio**, Ollama front-ends, or any
Streamable-HTTP MCP client. Written in Rust on top of the official
[`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) SDK; compiles to a
single binary.

---

## Intent

The goal is to let a locally-hosted model **search and read code from the open
internet without requiring the user to sign up for, pay for, or manage API
tokens.** Most "web search for LLMs" tools assume a Brave/Tavily/Exa/SerpAPI key.
Lodestone instead scrapes search engines (DuckDuckGo, Mojeek) and public,
keyless endpoints (raw GitHub, the StackExchange public API, the Wayback
Machine), and only reaches for credentials where the open web no longer offers
an alternative (optional GitHub token for code search).

Design principles:

- **Keyless by default.** Nothing here requires an account. The one optional
  credential is a GitHub token, because GitHub removed unauthenticated code
  search — and even then there's a keyless fallback.
- **Search *and* retrieve.** Finding a link is half the job; the model also needs
  to read the file, the page, or the answer. Retrieval tools are first-class.
- **Pluggable sources.** Every source implements one trait and is chosen and
  ordered via config — swap engines, add forges, reorder priority without code
  changes. See [CONTRIBUTING.md](CONTRIBUTING.md).
- **The model decides.** Rendering with a real browser is expensive, so it's a
  per-call flag the calling LLM can set when a site needs JavaScript or is
  bot-walling plain HTTP — not a fixed property of a provider.
- **Resilient.** Providers run as fallback chains or a merged meta-search, and
  page fetches fall back to the web archive when the live site is down or blocks
  us.

## Golden rules (non-negotiable)

The project's invariants live in one place — [docs/golden-rules.md](docs/golden-rules.md):

1. **Scrape is the default; render is optional** (model-controlled `render` flag).
2. **The LLM always decides** (rendering and what to retrieve next).
3. **Keyless by default** (credentials are strictly optional enhancements).
4. **Parallelize — always** (concurrent sourcing, never block the runtime).
5. **Everything is enable/disable-able** (tools, providers, and subsystems).
6. **Every provider is documented** (per-provider page + index + README row).

See [docs/golden-rules.md](docs/golden-rules.md) for the full statement of each.

---

## Tools

Tools come in two tiers — **general** (aggregated, everyday) and **per-provider**
(target one source). Every tool can be hidden via `[tools]`. `?` marks optional
arguments.

**Search** — query all configured providers of the kind (combined per
`[search].strategy`). All accept `render` to route through the headless browser.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `web_search` | `query`, `max_results?`, `render?` | General web search (DuckDuckGo, Mojeek, …). |
| `code_search` | `query`, `language?`, `max_results?`, `render?` | Source-code search across the configured forges (`[code].sites`). |
| `docs_search` | `query`, `max_results?` | Documentation & package registries (crates.io, npm, MDN). Keyless JSON APIs. |
| `qa_search` | `query`, `site?`, `max_results?`, `render?` | The configured Q&A providers (StackExchange network: StackOverflow, Server Fault, …). |

**Retrieve** — fetch one known thing.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `fetch_page` | `url`, `max_chars?` | Page → readable text over plain HTTP (the default reader). |
| `render_page` | `url`, `max_chars?` | Page → readable text via a headless browser (runs JS). |
| `fetch_repo_file` | `target`, `start_line?`, `end_line?` | A file from GitHub/GitLab/Gitea — blob/raw URL, or GitHub `owner/repo/path` (a `#L10-L40` fragment works too). |
| `wayback_fetch` | `url`, `timestamp?`, `max_chars?` | Archived snapshot from the Wayback Machine. |

**Meta**

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `list_providers` | — | Show the active providers, strategy, and ranking. |
| `hive_status` | — | Show the peer-to-peer hivemind graph (peers, reputation, edges); says disabled when off. |

**Per-provider** — one direct tool per *configured* provider, named `<kind>_<id>`
(e.g. `web_mojeek`, `code_github`, `qa_stackoverflow`), args `query`,
`max_results?`, `language?`, `site?`, `render?`. Targets a single source,
bypassing the chain/strategy. Generated from your config and gateable via
`[tools]`. StackOverflow adds one bespoke provider skill:
`qa_stackoverflow_answers` (`question`, `site?`, `max_answers?`) — read a
question's body + top answers (with code).

Typical flow: **search** → **retrieve** the best hit (`fetch_page` /
`render_page` / `fetch_repo_file` / `qa_stackoverflow_answers`).

---

## Quick start

Requires a recent Rust toolchain.

```sh
cargo run
```

The server listens on `http://127.0.0.1:8000/mcp` by default (and `GET /health`
returns `ok` for liveness checks). It's keyless out of
the box (DuckDuckGo, Mojeek, grep.app, StackExchange, raw GitHub, Wayback). The
headless browser is always compiled in; the Google engine and per-call
`render=true` additionally need a local **Chrome/Chromium** at runtime (only when
those paths are used).

### Add to LM Studio

Edit `%USERPROFILE%\.lmstudio\mcp.json` (or `~/.lmstudio/mcp.json`):

```json
{
  "mcpServers": {
    "lodestone": { "url": "http://127.0.0.1:8000/mcp" }
  }
}
```

(See `mcp.example.json`.)

### Docker

The image bundles Chromium:

```sh
docker compose up --build
# or
docker build -t lodestone-mcp .
docker run --rm -p 8000:8000 lodestone-mcp
```

Override the browser binary or behavior with env vars, e.g.
`-e LODESTONE_CHROME_PATH=/usr/bin/google-chrome`. Inside the container Chrome
runs as root, so `LODESTONE_CHROME_NO_SANDBOX=1` is set by default.

---

## Configuration

The repo ships a working, keyless configuration in [`config/`](config/) — clone
and run, no setup. It's a directory of small, granular files (one per concern,
plus one per provider under [`config/providers/`](config/providers/)), all
deep-merged in sorted path order. Edit those files, or drop a personal
`lodestone.toml` (gitignored) to override the baseline without touching them.
Complete alternative presets live in [`examples/`](examples/) (e.g.
`aggregate-all.toml`, `retrieval-only.toml`, `locked-down.toml`, `docker.toml`).

Precedence (low → high): built-in defaults < `config/**.toml`
(`$LODESTONE_CONFIG_DIR`) < `lodestone.toml` (`$LODESTONE_CONFIG`) < env vars.

The full schema, as a single annotated block:

```toml
bind = "127.0.0.1:8000"
auth_token = ""          # bearer token for /mcp; empty = open (set when on 0.0.0.0)

[tools]
enabled = []   # which tools to expose; empty = all
disabled = []  # applied after `enabled`

[search]
strategy = "fallback"   # or "aggregate" (merge/re-rank across providers)
ranking  = "composite"   # composite | reciprocal | borda | breadth | interleave
timeout_secs = 25        # per-request HTTP timeout (one short retry on failure)
# [search.engine_weights]  duckduckgo = 1.0   # composite per-engine weights
# trusted_domains = ["internal.docs.corp"]    # extra authority-boosted domains
# Optional per-kind overrides (empty = inherit the global values above):
# [search.web]  → strategy = "aggregate"
# [search.qa]   → strategy = "fallback"

[providers]
web  = ["duckduckgo", "mojeek"]
code = ["grep_app", "duckduckgo", "mojeek"]   # add "github", "google"
qa   = ["stackoverflow"]                       # render=true scrapes SO instead

[code]
sites = ["github.com"]   # add "gitlab.com", "codeberg.org", "gitea.com", …

[stackexchange]
default_site = "stackoverflow"

[google]
chrome_path = ""         # empty = auto-detect
no_sandbox  = false      # true inside containers
args        = []

[github]
token = ""               # enables the authenticated `github` code provider

[searxng]
url = ""                 # self-hosted SearXNG base URL; empty = provider off

[cache]
enabled = true           # cache search results in memory (cleared on restart)
ttl_secs = 300           # freshness window
max_entries = 512        # memory bound

[network]                # opt-in peer-to-peer hivemind (see docs/hivemind.md)
enabled = false
peers = []               # static peer base URLs (also gossiped + mDNS-discovered)
mdns = true              # LAN auto-discovery (when enabled)
token = ""               # optional shared secret for /hive endpoints
min_agreement = 2        # peers needed to trust a result without local search
relay_hops = 1           # forward a query a hop or two across the mesh (max 2)
state_file = ""          # persist peer reputations across restarts (path)

# Register self-hosted forges as keyless code providers (then add the id to
# [providers].code). See config/04-forges.toml.
# [forges.myhost]
# kind = "gitea"          # "gitlab" or "gitea" URL layout
# domain = "git.example.com"
```

Env overrides include `LODESTONE_BIND`, `LODESTONE_AUTH_TOKEN`,
`LODESTONE_SEARCH_STRATEGY`,
`LODESTONE_SEARCH_RANKING`, `LODESTONE_SEARCH_TIMEOUT_SECS`,
`LODESTONE_WEB_PROVIDERS`, `LODESTONE_CODE_PROVIDERS`, `LODESTONE_QA_PROVIDERS`,
`LODESTONE_CODE_SITES`, `LODESTONE_STACKEXCHANGE_SITE`,
`LODESTONE_STACKEXCHANGE_KEY`, `LODESTONE_STACKEXCHANGE_ALLOWED_SITES`,
`LODESTONE_CHROME_PATH`, `LODESTONE_CHROME_NO_SANDBOX`, `LODESTONE_CHROME_ARGS`,
`LODESTONE_GITHUB_TOKEN` / `GITHUB_TOKEN`, `LODESTONE_SEARXNG_URL`, and
`LODESTONE_CACHE_ENABLED` / `LODESTONE_CACHE_TTL_SECS` /
`LODESTONE_CACHE_MAX_ENTRIES`, and `LODESTONE_NETWORK_ENABLED` /
`LODESTONE_NETWORK_PEERS` / `LODESTONE_NETWORK_MDNS` / `LODESTONE_NETWORK_TOKEN` /
`LODESTONE_NETWORK_NODE_ID` / `LODESTONE_NETWORK_STATE_FILE`.

### Authentication

By default `/mcp` is unauthenticated (fine for the local `127.0.0.1` default).
When exposing the server beyond localhost (e.g. binding to `0.0.0.0` in a
container or on a LAN), set `auth_token` (or `LODESTONE_AUTH_TOKEN`): every `/mcp`
request must then send `Authorization: Bearer <token>` or it's rejected with
`401`. The token is compared in constant time, and `/health` stays open for
liveness probes.

### Tools (skills)

Each tool is an independent, modular capability. By default all are exposed;
restrict them with `[tools]` (or `LODESTONE_TOOLS_ENABLED` /
`LODESTONE_TOOLS_DISABLED`, comma-separated). `enabled` is an allowlist (empty =
all); `disabled` is applied afterward. Filtering affects both `tools/list` and
dispatch — a hidden tool returns "tool not found". For example, a
retrieval-only deployment:

```toml
[tools]
enabled = ["fetch_page", "fetch_repo_file", "wayback_fetch"]
```

### Strategies

- **fallback** (default) — try providers in order; the first non-empty result
  set wins. Fewer requests, lower latency.
- **aggregate** — query every provider for a kind concurrently, dedupe by URL,
  and re-rank, tagging which engines found each result. Broader coverage at the
  cost of more requests.

The aggregate re-ranking is configurable via `[search].ranking`
(`LODESTONE_SEARCH_RANKING`):

- **composite** (default) — a multi-signal fusion that goes beyond a
  weighted-position sum (what SearXNG-style mergers do): **weighted Reciprocal
  Rank Fusion** (canonical k=60, more robust than 1/(rank+1)) × **cross-engine
  consensus** × **lexical relevance** (query-term coverage in title/snippet) ×
  **authority** (HTTPS, trusted domains, resolved code, Q&A votes), then
  **domain-diversified** with an MMR-style decay so one site can't monopolize the
  top results. Tunable via `[search.engine_weights]` and `[search].trusted_domains`.
- **reciprocal** — Σ 1/(rank+1): rewards high placement and cross-engine agreement.
- **borda** — Σ (N − rank): linear positional scoring.
- **breadth** — consensus: rank by how many engines returned a result (best
  position breaks ties); resists single-engine noise.
- **interleave** — round-robin: each engine's 1st, then 2nd, …; maximizes
  source diversity.

Full reference (formulas, signals, tuning): [docs/ranking.md](docs/ranking.md).

Strategy and ranking can be set **per kind** with `[search.web]` /
`[search.code]` / `[search.qa]` (empty fields inherit the global `[search]`
values) — e.g. aggregate web/code for coverage while keeping qa on fallback so
the StackExchange API isn't hit on every query. Every request also has a shared
`[search].timeout_secs` cap and a single short-backoff retry on transient
failures.

### Providers

Detailed per-provider reference: [docs/providers.md](docs/providers.md).

| Kind | id | Notes |
| --- | --- | --- |
| web | `duckduckgo` | DuckDuckGo lite scrape. Rate-limits by IP. |
| web | `mojeek` | Independent index; tolerant of automation. |
| web | `medium` | Recent Medium articles for the query (treated as a tag) via RSS. |
| web/code | `searxng` | Self-hosted [SearXNG](https://docs.searxng.org/) metasearch JSON API. Keyless; needs `[searxng].url`. |
| web | `brave` | Brave Search API. Optional/**keyed** — off unless `[brave].key` is set. |
| web | `google_cse` | Google Programmable Search. Optional/**keyed** — needs `[google_cse].key` + `.cx`. |
| web/code | `google` | Headless-Chrome scrape. Needs a local Chrome at runtime; CAPTCHA-prone on datacenter IPs. |
| code | `grep_app` | grep.app JSON API (often bot-walled → empty). |
| code | `duckduckgo` / `mojeek` | Generic, `site:`-scoped to `[code].sites`. |
| code | `github` | Composite: keyless GitHub web scrape by default; uses the authenticated API when a token is set. |
| code | `gitlab` / `codeberg` / `gitea` | Keyless per-forge code search — one file per forge sharing an abstract `ForgeCodeProvider` (declarative `ForgeSpec`: domain + blob-URL parser). |
| qa | `stackoverflow` | StackExchange API (keyless; optional key raises quota). With `render=true`, scrapes stackoverflow.com via headless browser instead. |
| docs | `cratesio` | Rust crate index — keyless `crates.io` JSON search. |
| docs | `npm` | Node package index — keyless `registry.npmjs.org` JSON search. |
| docs | `mdn` | MDN Web Docs reference — keyless JSON search. |
| docs | `rubygems` / `packagist` / `nuget` / `hex` | Opt-in keyless registries (Ruby, PHP, .NET, Elixir). |
| docs | `aur` / `dockerhub` / `archlinux` | Opt-in keyless registries (Arch AUR, Docker images, Arch packages). |

**Self-hosted forges.** Register private GitLab/Gitea instances under `[forges]`
(see `config/04-forges.toml`): each `[forges.<id>] kind = "gitlab"|"gitea",
domain = "git.example.com"` becomes a keyless code provider once `<id>` is added
to `[providers].code`, and gets a `code_<id>` tool. It reuses the same
site-scoped search + blob-URL parsing as the built-in forges, on your host.

### Rendering (model-controlled)

`web_search`, `code_search`, `qa_search` (and the per-provider `<kind>_<id>`
tools) accept `render: true`, and `render_page` is the dedicated page-render skill. When used, the work goes
through a **shared, persistent headless Chrome** instead of plain HTTP — useful
for JS-heavy pages or to slip past rate-limits/bot-walls. It needs a local
Chrome/Chromium at runtime, and it's
slower, so it's left to the model to request per call.

### Caching

Search results (web/code/qa, general and per-provider) are cached in memory keyed
by the normalized query, so repeated identical searches don't re-hit
rate-limited engines or burn API quota. It's on by default with a 300s TTL
(`[cache]` / `LODESTONE_CACHE_*`), cleared on restart, holds only result lists
(never secrets), and never caches empty results — so a transiently blocked source
is retried rather than pinned empty. Retrieval tools (`fetch_page`, `render_page`,
`fetch_repo_file`, `wayback_fetch`, `qa_stackoverflow_answers`) are cached too, in
a separate store keyed by the request — so it never enters peer digests — under
the same `[cache]` settings.

### Hivemind (peer-to-peer)

An **opt-in** layer (`[network]`, off by default) where instances consult each
other's caches before scraping — spreading load and softening rate limits, while
staying a *helper*, never a dependency (zero peers = normal local search). Peers
are found via a static list and/or **mDNS** LAN discovery, then **gossip** their
known peers so the mesh grows from a seed, and can **relay** a query a hop or two
across the graph when a holder isn't directly reachable. Only *hashes* of queries
cross the wire (never raw text); responses carry only cached search results (never
secrets). Peer data is untrusted: a result is reused without a local search only
when `min_agreement` peers corroborate it, each peer's influence is capped, and
peers are weighted by an earned reputation (optionally persisted via
`state_file`) — so one bad node can't poison results. Inspect the mesh with the
`hive_status` tool. Full design + a two-node test:
[docs/hivemind.md](docs/hivemind.md).

### Forges (GitLab, Gitea, …)

`code_search` is scoped to the domains in `[code].sites` using the `site:`
operator, so adding `gitlab.com`, `codeberg.org`, or any Gitea host searches
them alongside GitHub through the same providers (and honors `render`). GitHub
itself dropped unauthenticated code search; set a `[github].token` to use the
authenticated `github` provider, otherwise the keyless site-scoped web search is
used. `fetch_repo_file` retrieves raw files from GitHub, GitLab, and Gitea
(blob/raw URL, or GitHub `owner/repo/path`).

---

## How Lodestone compares

Lodestone overlaps with several tools but targets a specific niche: **keyless,
code-aware, MCP-native, self-hosted.**

| | lodestone | SearXNG | Brave/Tavily/Exa MCP | `fetch` MCP | Firecrawl | GitHub MCP |
| --- | --- | --- | --- | --- | --- | --- |
| API key required | No¹ | No | **Yes** | No | **Yes** | **Yes** (token) |
| MCP-native | **Yes** | No (needs wrapper) | Yes | Yes | Yes | Yes |
| Web search | Yes (2 engines) | **Yes (~200 engines)** | Yes | No | partial | No |
| Code/forge search | **Yes** (GH/GL/Gitea) | No | No | No | No | GitHub only |
| Raw file retrieval | **Yes** | No | No | partial | Yes | Yes |
| Q&A (StackExchange) | **Yes** | via engines | No | No | No | No |
| Headless JS render | **On demand** | No | n/a (hosted) | No | Yes | n/a |
| Archive fallback | **Yes** | No | No | No | No | No |
| Self-hosted / offline-friendly | Yes (single binary) | Yes (Python+Redis) | No (SaaS) | Yes | No (SaaS) | partial |
| Result breadth / ranking | **Strong** (composite: RRF + consensus + relevance + authority + diversity) | Strong | Strong | n/a | Strong | n/a |

¹ Optional GitHub token for authenticated code search; everything else keyless.

**When to prefer something else:**

- **SearXNG** — you want the broadest, best-ranked general web search and don't
  mind running Python + Redis and wrapping it for MCP. (You can even point
  Lodestone-style usage at a SearXNG instance.) Lodestone trades breadth for
  being code-aware, MCP-native, and a single binary.
- **Brave / Tavily / Exa MCP** — you're fine with an API key and want
  managed, high-quality search/answers. Lodestone's pitch is *no key*.
- **Firecrawl** — you need robust large-scale crawling/extraction. Lodestone's
  rendering is single-page and best-effort.
- **Official GitHub MCP** — you live in GitHub (issues/PRs/repos) with a token.
  Lodestone is multi-forge and keyless-first, focused on *search + read*.

**Honest limitations:** scraping is brittle and breaks when sites change markup;
DuckDuckGo/Google aggressively rate-limit or CAPTCHA datacenter IPs; ranking is
simplistic; the StackExchange keyless API has a daily quota; the headless
browser adds latency and a Chrome dependency. Lodestone leans on fallback chains
and the web archive to stay useful despite this.

---

## Roadmap

Planned work and known gaps are tracked in [TODO.md](TODO.md).

## License

MIT.
