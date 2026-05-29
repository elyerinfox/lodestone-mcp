# lodestone-mcp

A **keyless-by-default, self-hosted [MCP](https://modelcontextprotocol.io) server**
that gives a local LLM a broad, composable toolkit — **search and retrieve** the open
web and developer ecosystem, **operate** the machine it runs on (Docker, Kubernetes,
files, shell, git, databases, serial/printers), and **compute** over real data
(math, geo, finance, units, dates, JSON/YAML/regex, NASA/space, markets) — all
without signing up for, paying for, or managing API keys.

It scrapes search engines and reads public, keyless endpoints instead of calling
paid, key-gated APIs, and talks to local daemons/devices directly. Built for local
runners like **LM Studio**, Ollama front-ends, or any Streamable-HTTP MCP client.
Written in Rust on the official [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk)
SDK; compiles to a single binary.

> **"Keyless by default" — what that means.** Everything works with **zero**
> accounts or keys. A few sources can *optionally* use a credential to unlock or
> improve them — the keyed web engines `brave`/`google_cse`, a GitHub token, a
> StackExchange key, a NASA key, a database URL. Each is **strictly optional and off
> unless you supply it**; none is ever required, and credentials are never logged or
> committed.

## Why "lodestone"?

It started as a small "search the web, retrieve code & docs" helper and kept
growing — into web/code/docs/Q&A search, page/PDF/file/archive retrieval, GitHub &
container lookups, local Docker/Kubernetes/filesystem/shell/git/database control,
host & GPU info, math/geo/finance/units/date/translation utilities, NASA/markets/
satellite data, and an opt-in peer-to-peer cache. That sprawl is the point.

This project was born out of frustration: getting a local model to actually *do*
things meant gluing together a dozen single-purpose tools, each with its own
ecosystem, install dance, auth, and quirks, just to assemble a workable toolkit.
The need it answers is for one **monolithic** solution — broad enough to cover the
surface area, yet **intelligent enough not to become a burden** itself. Keyless by
default, gated, and safe-by-construction, so adopting it costs a config line, not a
maintenance project.

The name fits. A **lodestone** is a naturally magnetized piece of magnetite — the
original compass, the very stone early navigators used to find north. That is what
this aims to be for a model: a single point that **draws scattered capabilities
together** and **orients** the model toward the right tool for the task at hand.
One stone, many bearings.

## What it is

- **A keyless toolkit for a local model** — one MCP server exposing ~100 small,
  composable [skills](docs/skills.md), each gateable.
- **Search _and_ retrieve.** Finding a link is half the job; reading the page, file,
  PDF, or answer is the other half. Retrieval is first-class.
- **Local-system aware.** Beyond the web, it can inspect and operate the host:
  containers, clusters, files, processes, git repos, databases, devices, GPU.
- **Safe by construction.** Destructive actions never fire unguarded (a confirm-token
  handshake), dangerous families are off by default, and credentials stay optional.
- **A single binary you run yourself.** No SaaS, no account, offline-friendly.

## What it isn't

- **Not a hosted/keyed search API** — keyless by default; keyed providers are
  optional add-ons.
- **Not a large-scale crawler** — rendering is single-page, on demand.
- **Not an agent framework** — it's the *tools*; your MCP host/model is the agent.
- **Not a guaranteed-stable data source** — scraping is best-effort and degrades to
  fallbacks / the web archive. See the [honest limitations](docs/comparison.md#honest-limitations).

## How it works

Lodestone speaks MCP over **Streamable HTTP** at `/mcp`. Search sources are
`SearchProvider`s grouped by *kind* (`web`/`code`/`qa`/`docs`) and combined by a
**strategy** (`fallback` or concurrent `aggregate` + re-rank); a per-call **`render`**
flag routes scraping through a shared headless Chrome. Everything else is a
self-contained **skill** module. Results and fetched files are cached (in-memory,
optionally Redis, and an on-disk file store). Adding a capability means adding a skill
or a provider, never editing `main.rs` — see [CONTRIBUTING.md](CONTRIBUTING.md) and
the [golden rules](docs/golden-rules.md).

## What it enables

Because the skills compose, a model can chain them into real, multi-step work. A few
concrete examples, by field:

- **Research & academia.** "Find the *Attention Is All You Need* paper and summarize
  its method" → `arxiv_search` → `read_pdf` on the free PDF (shared across your
  constellation so you don't re-download). Look up an **RFC** (`rfc_get`) or a **NIST/IEEE
  standard** (`standards_search`), read a **Wikipedia** article, or pull a
  reproducibility dataset/model card from **Hugging Face** (`hf_model`).
- **Astronomy & aerospace.** "When is the ISS next over Berlin?" → `sat_tle "ISS"` →
  `sat_observe` from your coordinates for azimuth/elevation/range; `sat_position` for
  the live ground sub-point. Pair with **NASA** APOD / near-Earth-object / Mars-rover
  data (`nasa_*`).
- **Physics & engineering.** Convert frequency ↔ wavelength ↔ period
  (`wave_frequency`), great-circle distance and bearing between coordinates
  (`geo_distance`/`geo_azimuth`), arbitrary unit conversions (`convert_units`), and
  evaluate/solve formulas (`math_eval`/`math_solve`).
- **DevOps & SRE.** Triage a box without a shell: `docker_ps` → `docker_logs`,
  `k8s_get` → `k8s_logs` → `k8s_scale`, `system_info`/`system_disks`/`system_gpu`,
  `git_run`, and a guarded `db_query`/`redis_command` to inspect state. Destructive
  steps (delete/remove/exec) pause for confirmation.
- **Software development.** `code_search` across GitHub/GitLab/Gitea →
  `fetch_repo_file` to read the exact source; `docs_search` across crates.io/npm/MDN
  and framework docs; `github_releases` to summarize what changed.
- **Finance & data.** `currency_convert` (live ECB rates), `compound_interest` /
  `loan_payment`, `stock_quote`, and JSON/YAML/`regex` wrangling for ad-hoc data.
- **Self-hosted fleets.** Run several instances with the opt-in
  [constellation](docs/constellation.md): a result or PDF one node fetched is served to the
  others (content-verified, hash-only on the wire) — dodging the rate limits of
  arXiv, IETF, search engines, and friends.

The full, exhaustive lists: **[skills](docs/skills.md)** · **[tools](docs/tools.md)**
· **[providers](docs/providers.md)**.

## Features

- **Search:** web, source code (multi-forge), documentation & package registries,
  framework/tooling docs, and Q&A (StackExchange).
- **Retrieve:** readable page text (HTTP or headless render), repo files, PDFs (read
  + generate), and Wayback snapshots — with an on-disk file store + cache.
- **Knowledge (keyless):** GitHub metadata, RFCs, standards (IEEE/SAE/NIST/ISO),
  arXiv, Hugging Face, Wikipedia, kernel.org.
- **Containers & cloud-native (keyless):** Docker Hub / OCI registry / Artifact Hub
  lookups. See [docs/containers.md](docs/containers.md).
- **Local system:** Docker daemon, Kubernetes, filesystem, shell, git, host/GPU
  info, and Postgres/MySQL/Redis clients — gated, with destructive actions guarded.
- **Compute & utilities:** date/time + timezones, translation, JSON/YAML/regex,
  math/algebra, geo distance/bearing, wave/frequency, finance, currency, units.
- **Space, markets & science (keyless):** NASA open data, stock/FX quotes, satellite
  (SGP4) tracking.
- **Devices (off by default):** raw serial I/O and OS printing.
- **Resilience & ops:** composite ranking, in-memory/Redis cache + file store,
  optional bearer auth, and an opt-in peer-to-peer [constellation](docs/constellation.md).

## Quick start

Requires a recent Rust toolchain.

```sh
cargo run
```

Listens on `http://127.0.0.1:8000/mcp` (and `GET /health` returns `ok`). Keyless out
of the box. The headless browser is always compiled in; the `google` engine and
per-call `render=true` additionally need a local **Chrome/Chromium** at runtime.

**LM Studio** — add to `%USERPROFILE%\.lmstudio\mcp.json` (or `~/.lmstudio/mcp.json`):

```json
{ "mcpServers": { "lodestone": { "url": "http://127.0.0.1:8000/mcp" } } }
```

(See `mcp.example.json`.)

**Docker** — the image bundles Chromium:

```sh
docker compose up --build      # or: docker build -t lodestone-mcp . && docker run --rm -p 8000:8000 lodestone-mcp
```

## Configuration

The repo ships a working, keyless config in [`config/`](config/) (granular files,
deep-merged); override it with a gitignored `lodestone.toml` or `LODESTONE_*` env
vars. Local-system families (`[filesystem]`, `[shell]`, `[serial]`, `[printer]`,
`[store]`, `[databases.*]`) are **off by default**. Full schema, env vars, auth,
strategies, caching, forges/doc-sites: **[docs/configuration.md](docs/configuration.md)**.

## Documentation

| Doc | What's in it |
| --- | --- |
| [skills.md](docs/skills.md) | Every skill family, grouped, with a page each. |
| [tools.md](docs/tools.md) | Every tool, its arguments, and purpose. |
| [providers.md](docs/providers.md) | Every search provider, by family, with a page each. |
| [configuration.md](docs/configuration.md) | Full config schema, env vars, auth, strategies, caching. |
| [ranking.md](docs/ranking.md) | The composite ranker: signals, formulas, tuning. |
| [containers.md](docs/containers.md) | Docker Hub / OCI / Artifact Hub lookups. |
| [constellation.md](docs/constellation.md) | The opt-in peer-to-peer layer (results + blob sharing). |
| [golden-rules.md](docs/golden-rules.md) | The project's invariants. |
| [comparison.md](docs/comparison.md) | How Lodestone compares; limitations. |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Architecture and how to add a skill/provider. |

## Constellation — share the load, be a good neighbor

Lodestone reaches the open web by scraping search engines and fetching from
rate-limited sources (arXiv, IETF, registries, …). Those limits are almost always
enforced **per IP**, not per user — so when several people share an uplink (an
office, a lab, a campus, a VPN, a household behind one NAT), every redundant scrape
you make spends a budget your neighbors also draw on. Hammer DuckDuckGo from a
shared egress and *everyone* behind that address starts seeing tarpits and 403s,
not just you. The cost of one greedy node is paid by the whole network.

The opt-in [**constellation**](docs/constellation.md) turns that dynamic around. When you
enable it, your instance first asks its peers whether one of them has *already*
fetched a query or file before it goes to the open web:

- **Fewer requests per IP.** A result or PDF that any one node retrieves is served
  to the others, so the group hits the rate-limited source once instead of N times.
  You stop competing with your colleagues for the same shrinking budget — and stop
  being the reason their searches start failing.
- **You give as much as you get.** Every node both consults and serves: the cache
  you fill from your own work softens the next person's load, and theirs softens
  yours. A shared connection becomes a reason the experience gets *better* as more
  people join, not worse.
- **Privacy-preserving by design.** Only *hashes* of query keys cross the wire
  (never raw query text), responses carry only already-public web results/bytes
  (never secrets), peer data is trusted only by content-verified consensus, and the
  `/constellation` endpoints can require a shared `[network].token`. It stays strictly
  opt-in and is never a dependency — local search works with zero peers.

If you run more than one instance, or share a network with others who do, please
consider turning it on for your peers' sake: set `[network].enabled = true` (LAN
peers are found automatically over mDNS; add `[network].peers` for off-LAN nodes).
See [`config/06-network.toml`](config/06-network.toml) and
[docs/constellation.md](docs/constellation.md).

## Golden rules

The project's non-negotiable invariants live in
[docs/golden-rules.md](docs/golden-rules.md): scrape-by-default/render-optional · the
LLM decides · keyless by default · always parallelize · everything is
enable/disable-able · every provider/skill is documented · every tool is a
self-contained skill module · **destructive actions never fire unguarded**.

## Disclaimer

**No warranty.** Lodestone is provided "AS IS", without warranty of any kind, express
or implied. In no event shall the authors be liable for any claim, damages, or
liability arising from its use (this restates the MIT [LICENSE](LICENSE), which
governs).

**Use at your own risk.** Lodestone scrapes third-party sites and calls public
endpoints — you are responsible for complying with their terms and for any
rate-limiting that results. Its **local-system** tools are powerful: the Docker,
Kubernetes, filesystem, shell, git, database, serial, and printer families act on
your real machine, daemon, cluster, devices, and data. They are **gated** (the most
dangerous off by default) and **destructive actions require a confirmation step**,
and they're meant to run behind an MCP host that approves calls — review what you
enable, scope credentials/contexts narrowly, and prefer read-only or non-production
targets when in doubt. You are responsible for everything the model does through them.

## Roadmap & license

Planned work and known gaps: [TODO.md](TODO.md). Licensed **MIT** (see
[LICENSE](LICENSE)).

## Supporting the project

Lodestone is free, open-source, and keyless by design — there is nothing to buy and
no account to create. It is developed and maintained in spare time.

If lodestone has helped you finally get genuine, practical use out of running local
LLMs, please consider chipping in a few dollars toward its continued development and
upkeep via [GitHub Sponsors](https://github.com/sponsors/elyerinfox). Contributions
are entirely voluntary and never gate any feature — every capability remains
available to everyone, sponsor or not. Non-financial support is just as valued:
starring the repo, filing thoughtful issues, and contributing fixes or new
skills/providers all help the project thrive.
