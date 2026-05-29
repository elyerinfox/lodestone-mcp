# lodestone-mcp

A **keyless-by-default, self-hosted [MCP](https://modelcontextprotocol.io) server**
that gives a local LLM a broad toolkit for working with the open web and the
developer ecosystem — **searching** it, **retrieving** from it, and **inspecting**
it — without requiring you to sign up for, pay for, or manage any API keys.

It scrapes search engines and reads public, keyless endpoints instead of calling
paid, key-gated APIs. Built for local runners like **LM Studio**, Ollama
front-ends, or any Streamable-HTTP MCP client. Written in Rust on the official
[`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) SDK; compiles to a
single binary.

> **"Keyless by default" — what that means.** Everything works with **zero**
> accounts or keys. A few providers can *optionally* use a credential to unlock or
> improve a source — the keyed web engines `brave` and `google_cse`, a GitHub token
> (authenticated code search / higher rate limits), and a StackExchange key (higher
> quota). Each is **strictly optional and off unless you supply the key**; none is
> ever required, and credentials are never logged or committed. So lodestone is
> keyless *by default*, not key-*less* in the sense of forbidding keys.

> Started as "search the web and retrieve code & docs" and grew into a general
> keyless research/retrieval toolbox: web/code/docs/Q&A search, page/PDF/file/
> archive retrieval, GitHub metadata, container & cloud-native lookups (Docker
> Hub, OCI registries, Artifact Hub), plus date/time and translation utilities.

## What it is

- **A keyless web + developer-ecosystem toolbox for a local model.** One MCP server
  exposing many small, composable skills — see the [tools reference](docs/tools.md).
- **Search _and_ retrieve.** Finding a link is half the job; reading the page, file,
  PDF, or answer is the other half. Retrieval is first-class.
- **Pluggable and configurable.** Every source implements one trait and is chosen
  and ordered via config — swap engines, add forges/doc-sites, gate any tool, with
  no code changes.
- **A single binary you run yourself.** No SaaS, no account, runs offline-friendly.

## What it isn't

- **Not a hosted/keyed search API** (Brave/Tavily/Exa) — it's keyless by default;
  keyed providers are optional add-ons, never the baseline (see the note above).
- **Not a large-scale crawler** (Firecrawl) — rendering is single-page, on demand.
- **Not a GitHub/issues client** — it's multi-forge and focused on *search + read*.
- **Not a guaranteed-stable data source** — scraping is best-effort and degrades
  to fallbacks/the web archive when a site blocks or changes. See the
  [honest limitations](docs/comparison.md#honest-limitations).

## How it works

Lodestone speaks MCP over **Streamable HTTP** at `/mcp`. Each data source is a
`SearchProvider` grouped by *kind* (`web` / `code` / `qa` / `docs`); the registry
combines a kind's providers by a **strategy** — `fallback` (first non-empty wins)
or `aggregate` (concurrent meta-search, then a re-rank). Scraping over plain HTTP
is the default; a per-call **`render`** flag routes through a shared headless Chrome
for JS-heavy or bot-walled pages. Results are cached in memory. Beyond search,
standalone skills hit keyless public APIs directly (GitHub, Docker Hub, OCI
registries, Artifact Hub, Google Translate). Architecture details and how to add a
provider: [CONTRIBUTING.md](CONTRIBUTING.md).

## Features

- **Search:** web, source code (multi-forge), documentation & package registries,
  framework/tooling docs, and Q&A (StackExchange).
- **Retrieve:** readable page text (HTTP or headless render), repo files, PDFs
  (read + generate), and Wayback Machine snapshots.
- **GitHub (keyless):** release notes, user/org profiles, repo metadata.
- **Containers & cloud-native (keyless):** Docker Hub search/metadata/tags,
  tag/manifest inspection on **any** OCI registry (Docker Hub, GHCR, Quay, …), and
  Artifact Hub (Helm/Operators/krew/policies). See [docs/containers.md](docs/containers.md).
- **Utilities:** date/time + timezone math, and translation/language detection.
- **Resilience & ops:** composite ranking, in-memory cache, optional bearer auth,
  and an opt-in peer-to-peer [hivemind](docs/hivemind.md).

Full, exhaustive lists: **[tools](docs/tools.md)** · **[providers](docs/providers.md)**.

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
vars. Presets live in [`examples/`](examples/). Full schema, env vars, auth,
strategies, caching, forges/doc-sites: **[docs/configuration.md](docs/configuration.md)**.
Ranking internals: [docs/ranking.md](docs/ranking.md).

## Providers

Sources are grouped into spec-driven families (engines, forges, registries, doc
sites, …) and selected per kind in `config/02-search.toml`. The exhaustive,
per-provider reference is **[docs/providers.md](docs/providers.md)**.

## Golden rules

The project's non-negotiable invariants live in one place,
[docs/golden-rules.md](docs/golden-rules.md): scrape-by-default/render-optional ·
the LLM decides · keyless by default · always parallelize · everything is
enable/disable-able · every provider is documented · every tool is a self-contained
skill module.

## Documentation

| Doc | What's in it |
| --- | --- |
| [tools.md](docs/tools.md) | Every tool, its arguments, and purpose. |
| [providers.md](docs/providers.md) | Every provider, by family, with per-provider pages. |
| [configuration.md](docs/configuration.md) | Full config schema, env vars, auth, strategies, caching. |
| [ranking.md](docs/ranking.md) | The composite ranker: signals, formulas, tuning. |
| [containers.md](docs/containers.md) | Docker Hub / OCI / Artifact Hub tools. |
| [hivemind.md](docs/hivemind.md) | The opt-in peer-to-peer layer. |
| [golden-rules.md](docs/golden-rules.md) | The project's invariants. |
| [comparison.md](docs/comparison.md) | How Lodestone compares; limitations. |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Architecture and how to add a provider. |

## How it compares

Keyless, code-aware, MCP-native, self-hosted — versus SearXNG, the keyed search
MCPs, `fetch`, Firecrawl, and the GitHub MCP. Side-by-side table and "when to
prefer something else": [docs/comparison.md](docs/comparison.md).

## Disclaimer

**No warranty.** Lodestone is provided "AS IS", without warranty of any kind,
express or implied, including but not limited to the warranties of merchantability,
fitness for a particular purpose, and noninfringement. In no event shall the authors
or copyright holders be liable for any claim, damages, or other liability arising
from, out of, or in connection with the software or its use (this restates the MIT
[LICENSE](LICENSE), which governs).

**Use at your own risk.** Lodestone scrapes third-party websites and calls public
endpoints; you are responsible for using it in compliance with those services'
terms and applicable law, and for any rate-limiting/blocking that results. The
optional local-system tools are powerful: the **Docker** (`[docker]`) and
**Kubernetes** (`[kubernetes]`) families act on your real daemon and cluster, and
when destructive actions are enabled they can **stop/remove containers or delete
cluster resources**. They are gated (destructive actions off by default) and meant
to run behind an MCP host that approves calls — review what you enable, scope
credentials/contexts narrowly, and prefer read-only or non-production targets when
in doubt. You are responsible for all actions the model takes through these tools.

## Roadmap & license

Planned work and known gaps: [TODO.md](TODO.md). Licensed **MIT** (see
[LICENSE](LICENSE)).
   
