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

---

## Tools

| Tool | Purpose |
| --- | --- |
| `web_search` | General web search (DuckDuckGo/Mojeek/…). `render` optional. |
| `code_search` | Source-code search across configured forges. `render` optional. |
| `github_fetch_file` | Fetch a full file from GitHub via `raw.githubusercontent.com` (URL, raw URL, or `owner/repo/path`; line ranges). |
| `fetch_page` | Fetch any URL → readable text. Archive fallback; `render` optional. |
| `wayback_fetch` | Read a page's archived snapshot from the Wayback Machine. |
| `stackexchange_search` | Search StackOverflow / StackExchange. |
| `stackexchange_answers` | Read a question's body and top answers (with code). |
| `list_providers` | Show the active providers and strategy. |

Typical flow: **search** (`web_search` / `code_search` / `stackexchange_search`)
→ **retrieve** (`github_fetch_file` / `fetch_page` / `stackexchange_answers`) on
the best hit.

---

## Quick start

Requires a recent Rust toolchain.

```sh
# Keyless build (DuckDuckGo, Mojeek, grep.app, StackExchange, raw GitHub, Wayback)
cargo run

# With the headless-browser provider (Google) + per-call rendering.
# Requires a local Chrome/Chromium.
cargo run --features google
```

The server listens on `http://127.0.0.1:8000/mcp` by default.

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

The image bundles Chromium and builds with the browser provider enabled:

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

Copy `lodestone.example.toml` to `lodestone.toml` (gitignored) or use env vars.
Precedence: built-in defaults < `lodestone.toml` (or `$LODESTONE_CONFIG`) < env.

```toml
bind = "127.0.0.1:8000"

[search]
strategy = "fallback"   # or "aggregate" (merge/re-rank across providers)

[providers]
web  = ["duckduckgo", "mojeek"]
code = ["grep_app", "duckduckgo", "mojeek"]   # add "github", "google"
qa   = ["stackoverflow"]                       # add "stackoverflow_scrape"

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

### Strategies

- **fallback** (default) — try providers in order; the first non-empty result
  set wins. Fewer requests, lower latency.
- **aggregate** — query every provider for a kind concurrently, dedupe by URL,
  and re-rank by a SearXNG-style score (Σ 1/rank), tagging which engines found
  each result. Broader coverage at the cost of more requests.

### Providers

| Kind | id | Notes |
| --- | --- | --- |
| web | `duckduckgo` | DuckDuckGo lite scrape. Rate-limits by IP. |
| web | `mojeek` | Independent index; tolerant of automation. |
| web | `medium` | Recent Medium articles for the query (treated as a tag) via RSS. |
| web/code | `google` | Headless Chrome. Needs `--features google` + Chrome. |
| code | `grep_app` | grep.app JSON API (often bot-walled → empty). |
| code | `duckduckgo` / `mojeek` | `site:`-scoped to `[code].sites`. |
| code | `github` | Authenticated GitHub code-search API. Needs a token. |
| qa | `stackoverflow` | StackExchange API (keyless; optional key raises quota). With `render=true`, scrapes stackoverflow.com via headless browser instead. |

### Rendering (model-controlled)

`web_search`, `code_search`, and `fetch_page` accept `render: true`. When set
(and the binary was built with `--features browser`/`google`), the HTML-scraping
providers fetch through a **shared, persistent headless Chrome** instead of plain
HTTP — useful for JS-heavy pages or to slip past rate-limits/bot-walls. It's
slower, so it's left to the model to request per call.

### Forges (GitLab, Gitea, …)

`code_search` is scoped to the domains in `[code].sites` using the `site:`
operator, so adding `gitlab.com`, `codeberg.org`, or any Gitea host searches
them alongside GitHub through the same providers (and honors `render`). GitHub
itself dropped unauthenticated code search; set a `[github].token` to use the
authenticated `github` provider, otherwise the keyless site-scoped web search is
used. `github_fetch_file` retrieves raw files from GitHub; for other forges use
`fetch_page` on the blob URL.

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

## License

MIT.
