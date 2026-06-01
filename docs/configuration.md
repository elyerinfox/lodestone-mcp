# Configuration

The repo ships a working, keyless configuration in [`config/`](../config/) — clone
and run, no setup. It's a directory of small, granular files (one per concern, plus
one per provider under [`config/providers/`](../config/providers/)), all
deep-merged in sorted path order. Edit those files, or drop a personal
`lodestone.toml` (gitignored) to override the baseline without touching them.
Complete alternative presets live in [`examples/`](../examples/) (e.g.
`aggregate-all.toml`, `retrieval-only.toml`, `locked-down.toml`, `docker.toml`).

**Precedence** (low → high): built-in defaults < `config/**.toml`
(`$LODESTONE_CONFIG_DIR`) < `lodestone.toml` (`$LODESTONE_CONFIG`) < environment
variables.

## Annotated schema

```toml
bind = "127.0.0.1:8000"
auth_token = ""          # bearer token for /mcp; empty = open (set when on 0.0.0.0)

[tools]
enabled = []   # which tools to expose; empty = all
disabled = []  # applied after `enabled`

[search]
strategy = "fallback"    # or "aggregate" (merge/re-rank across providers)
ranking  = "composite"   # composite | reciprocal | borda | breadth | interleave
timeout_secs = 25        # per-request HTTP timeout (one short retry on failure)
# [search.engine_weights]  duckduckgo = 1.0   # composite per-engine weights
# trusted_domains = ["internal.docs.corp"]    # extra authority-boosted domains
# Optional per-kind overrides (empty = inherit the global values above):
# [search.web]   → strategy = "aggregate"
# [search.docs]  → strategy = "aggregate"   (the shipped default for docs)
# [search.qa]    → strategy = "fallback"

[providers]
web  = ["duckduckgo", "mojeek"]
code = ["grep_app", "duckduckgo", "mojeek"]   # add "github", "google"
qa   = ["stackoverflow"]                       # render=true scrapes SO instead
docs = ["cratesio", "npm", "mdn", "php", "laravel", "vue", "react", "svelte",
        "docker", "kubernetes", "helm"]        # registries + framework/tooling docs

[code]
sites = ["github.com"]   # add "gitlab.com", "codeberg.org", "gitea.com", …

[stackexchange]
default_site = "stackoverflow"
# key = ""               # optional; raises the keyless quota (not a login)
# allowed_sites = []     # guardrail: restrict which SE sites may be queried

[retrieval]
default_chars = 16000    # text returned when a retrieval call omits max_chars
max_chars = 100000       # hard cap a retrieval tool may return

[google]
chrome_path = ""         # empty = auto-detect
no_sandbox  = false      # true inside containers
args        = []

[github]
token = ""               # enables the authenticated `github` code provider

[searxng]
url = ""                 # self-hosted SearXNG base URL; empty = provider off

# [brave]      key = ""            # optional KEYED web provider (off unless set)
# [google_cse] key = ""  cx = ""   # optional KEYED web provider (off unless set)

[cache]
enabled = true           # cache search results in memory (cleared on restart)
ttl_secs = 300           # freshness window
max_entries = 512        # memory bound

[network]                # opt-in peer-to-peer constellation (see constellation.md)
enabled = true           # default: participate, share cache, opt out per-feature below
peers = []               # static peer base URLs (also gossiped + mDNS-discovered)
mdns = true              # LAN auto-discovery (when enabled)
token = ""               # optional shared secret for /constellation endpoints
min_agreement = 2        # peers needed to trust a result without local search
relay_hops = 1           # forward a query a hop or two across the mesh (max 2)
state_file = ""          # persist peer reputations across restarts (path)
[network.capabilities]   # per-feature opt-in for what we OFFER peers (see constellation.md)
query     = true         # answer cache consults (the whole point of joining)
retrieval = false        # accept URL-fetching jobs (alias of legacy delegation_enabled)
blob      = true         # serve file-store blobs we already cached
browser   = false        # accept peer-hosted browser sessions (much higher trust surface)

# Register self-hosted forges as keyless code providers (config/04-forges.toml),
# then add the id to [providers].code:
# [forges.myhost]   kind = "gitea"   domain = "git.example.com"

# Register custom documentation sites (config/07-docsites.toml), then add the id
# to [providers].docs:
# [docsites.mydocs]   domain = "docs.example.com"
```

## Environment overrides

Every scalar/list has a `LODESTONE_*` env override (highest precedence):
`LODESTONE_BIND`, `LODESTONE_AUTH_TOKEN`, `LODESTONE_TOOLS_ENABLED` /
`LODESTONE_TOOLS_DISABLED`, `LODESTONE_SEARCH_STRATEGY` /
`LODESTONE_SEARCH_RANKING` / `LODESTONE_SEARCH_TIMEOUT_SECS`,
`LODESTONE_WEB_PROVIDERS` / `LODESTONE_CODE_PROVIDERS` / `LODESTONE_QA_PROVIDERS` /
`LODESTONE_DOCS_PROVIDERS`, `LODESTONE_CODE_SITES`, `LODESTONE_STACKEXCHANGE_SITE` /
`_KEY` / `_ALLOWED_SITES`, `LODESTONE_RETRIEVAL_DEFAULT_CHARS` / `_MAX_CHARS`,
`LODESTONE_CHROME_PATH` / `_NO_SANDBOX` / `_ARGS`,
`LODESTONE_GITHUB_TOKEN` / `GITHUB_TOKEN`, `LODESTONE_SEARXNG_URL`,
`LODESTONE_BRAVE_KEY`, `LODESTONE_GOOGLE_CSE_KEY` / `_CX`,
`LODESTONE_CACHE_ENABLED` / `_TTL_SECS` / `_MAX_ENTRIES`, and
`LODESTONE_NETWORK_ENABLED` / `_PEERS` / `_MDNS` / `_TOKEN` / `_NODE_ID` /
`_STATE_FILE`. List vars are comma-separated.

## Authentication

By default `/mcp` is unauthenticated (fine for the local `127.0.0.1` default). When
exposing the server beyond localhost (e.g. binding to `0.0.0.0` in a container or
on a LAN), set `auth_token` (or `LODESTONE_AUTH_TOKEN`): every `/mcp` request must
then send `Authorization: Bearer <token>` or it's rejected with `401`. The token is
compared in constant time, and `/health` stays open for liveness probes.

## Tools (skills)

Each tool is an independent capability. By default all are exposed; restrict them
with `[tools]` (or `LODESTONE_TOOLS_ENABLED` / `LODESTONE_TOOLS_DISABLED`).
`enabled` is an allowlist (empty = all); `disabled` is applied afterward. Filtering
affects both `tools/list` and dispatch — a hidden tool returns "tool not found".
Example, a retrieval-only deployment:

```toml
[tools]
enabled = ["fetch_page", "fetch_repo_file", "wayback_fetch"]
```

The full tool list is in [tools.md](tools.md).

## Strategies

- **fallback** (default) — try providers in order; the first non-empty result set
  wins. Fewer requests, lower latency.
- **aggregate** — query every provider for a kind concurrently, dedupe by URL, and
  re-rank. Broader coverage at the cost of more requests. (The shipped default for
  the `docs` kind.)

Strategy and ranking can be set **per kind** with `[search.web]` / `[search.code]`
/ `[search.qa]` / `[search.docs]` (empty fields inherit the global `[search]`). The
aggregate re-ranking method (`composite` by default, plus `reciprocal`/`borda`/
`breadth`/`interleave`) and its tuning are documented in full in
[ranking.md](ranking.md).

## Caching

Search results (general and per-provider) are cached in memory keyed by the
normalized query, so repeated identical searches don't re-hit rate-limited engines
or burn quota. On by default with a 300s TTL (`[cache]` / `LODESTONE_CACHE_*`),
cleared on restart, holding only result lists (never secrets), and never caching
empty results — so a transiently blocked source is retried rather than pinned
empty. Retrieval-tool output is cached too, in a separate store keyed by the
request (so it never enters peer digests), under the same `[cache]` settings.

## Forges & custom doc sites

`code_search` is scoped to `[code].sites` via the `site:` operator, so adding
`gitlab.com`, `codeberg.org`, or any Gitea host searches them alongside GitHub.
Register **private** GitLab/Gitea hosts under `[forges.<id>]`
([`config/04-forges.toml`](../config/04-forges.toml)) to get a keyless `code_<id>`
provider; register any **documentation host** under `[docsites.<id>]`
([`config/07-docsites.toml`](../config/07-docsites.toml)) to get a keyless
`docs_<id>` provider. See [providers.md](providers.md).

## Constellation (peer-to-peer)

The opt-in `[network]` layer is documented in full — design, wire protocol,
anti-poisoning, and a two-node test — in [constellation.md](constellation.md).
