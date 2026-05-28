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

1. **Scrape is the default; render is optional and a fallback.** Sources fetch
   over plain HTTP by default; the headless browser runs only when the model asks
   for it (a `render` flag, or the `render_page` tool). (The `google` engine is
   the one exception — it can't be scraped, so it's browser-only and opt-in.)
2. **The LLM always decides.** `render` is a per-call flag the model controls;
   the server never enables it on its own.
3. **Keyless by default.** Credentials (GitHub token, StackExchange key) are
   optional enhancements over a keyless fallback — never required.
4. **Parallelize — always.** Independent work runs concurrently; aggregate
   sourcing spawns each provider on its own task across the multi-threaded
   runtime, and no path blocks the runtime with sync I/O.

---

## Tools

| Tool | Purpose |
| --- | --- |
| `web_search` | General web search (DuckDuckGo/Mojeek/…). `render` optional. |
| `code_search` | Source-code search across configured forges. `render` optional. |
| `fetch_repo_file` | Fetch a full file from GitHub/GitLab/Gitea (blob/raw URL, or GitHub `owner/repo/path`; line ranges). |
| `fetch_page` | Fetch any URL → readable text over plain HTTP. |
| `render_page` | Fetch a URL → readable text via a headless browser (runs JS). |
| `wayback_fetch` | Read a page's archived snapshot from the Wayback Machine. |
| `stackexchange_search` | Search StackOverflow / StackExchange. |
| `stackexchange_answers` | Read a question's body and top answers (with code). |
| `list_providers` | Show the active providers and strategy. |

Typical flow: **search** (`web_search` / `code_search` / `stackexchange_search`)
→ **retrieve** (`fetch_repo_file` / `fetch_page` / `render_page` /
`stackexchange_answers`) on the best hit.

Tools come in two tiers:

- **General** (above) — the aggregated, everyday tools; each search tool queries
  all configured providers of its kind (per `[search].strategy`).
- **Per-provider** — one direct tool per *configured* provider, named
  `<kind>_<id>` (e.g. `web_mojeek`, `code_github`, `qa_stackoverflow`), to target
  a single source and bypass the chain/strategy. They're generated from your
  config and are gateable like any tool via `[tools]`.

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

[tools]
enabled = []   # which tools to expose; empty = all
disabled = []  # applied after `enabled`

[tools]
enabled = []   # which tools to expose; empty = all
disabled = []  # applied after `enabled`

[search]
strategy = "fallback"   # or "aggregate" (merge/re-rank across providers)

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
```

Env overrides include `LODESTONE_BIND`, `LODESTONE_SEARCH_STRATEGY`,
`LODESTONE_WEB_PROVIDERS`, `LODESTONE_CODE_PROVIDERS`, `LODESTONE_QA_PROVIDERS`,
`LODESTONE_CODE_SITES`, `LODESTONE_STACKEXCHANGE_SITE`,
`LODESTONE_STACKEXCHANGE_KEY`, `LODESTONE_STACKEXCHANGE_ALLOWED_SITES`,
`LODESTONE_CHROME_PATH`, `LODESTONE_CHROME_NO_SANDBOX`, `LODESTONE_CHROME_ARGS`,
and `LODESTONE_GITHUB_TOKEN` / `GITHUB_TOKEN`.

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

- **reciprocal** (default) — Σ 1/(rank+1): rewards high placement and
  cross-engine agreement.
- **borda** — Σ (N − rank): linear positional scoring.
- **breadth** — consensus: rank by how many engines returned a result (best
  position breaks ties); resists single-engine noise.
- **interleave** — round-robin: each engine's 1st, then 2nd, …; maximizes
  source diversity.

### Providers

Detailed per-provider reference: [docs/providers.md](docs/providers.md).

| Kind | id | Notes |
| --- | --- | --- |
| web | `duckduckgo` | DuckDuckGo lite scrape. Rate-limits by IP. |
| web | `mojeek` | Independent index; tolerant of automation. |
| web | `medium` | Recent Medium articles for the query (treated as a tag) via RSS. |
| web/code | `google` | Headless-Chrome scrape. Needs a local Chrome at runtime; CAPTCHA-prone on datacenter IPs. |
| code | `grep_app` | grep.app JSON API (often bot-walled → empty). |
| code | `duckduckgo` / `mojeek` | Generic, `site:`-scoped to `[code].sites`. |
| code | `github` | Composite: keyless GitHub web scrape by default; uses the authenticated API when a token is set. |
| code | `gitlab` / `codeberg` / `gitea` | Keyless per-forge code search — one file per forge sharing an abstract `ForgeCodeProvider` (declarative `ForgeSpec`: domain + blob-URL parser). |
| qa | `stackoverflow` | StackExchange API (keyless; optional key raises quota). With `render=true`, scrapes stackoverflow.com via headless browser instead. |

### Rendering (model-controlled)

`web_search`, `code_search`, and `stackexchange_search` accept `render: true`,
and `render_page` is the dedicated page-render skill. When used, the work goes
through a **shared, persistent headless Chrome** instead of plain HTTP — useful
for JS-heavy pages or to slip past rate-limits/bot-walls. It needs a local
Chrome/Chromium at runtime, and it's
slower, so it's left to the model to request per call.

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
| Result breadth / ranking | Basic | **Strong** | Strong | n/a | Strong | n/a |

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
