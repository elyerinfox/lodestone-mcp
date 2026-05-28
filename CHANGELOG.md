# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - unreleased

First release: a keyless MCP server that searches and retrieves code and docs from
the open web by scraping search engines and public endpoints (no API keys
required), served over Streamable HTTP at `/mcp`.

### Added

- **Tools.** General search (`web_search`, `code_search`, `qa_search`), retrieval
  (`fetch_page`, `render_page`, `fetch_repo_file`, `wayback_fetch`,
  `qa_stackoverflow_answers`), and `list_providers`. Plus one auto-generated
  per-provider tool per configured source (`<kind>_<id>`, e.g. `web_mojeek`,
  `code_github`, `qa_stackoverflow`). Every tool is independently gateable via
  `[tools]`.
- **Providers** across five families: engine (`duckduckgo`, `mojeek`, `google`),
  forge (`gitlab`, `codeberg`, `gitea`), registry (the `docs` kind, keyless JSON
  package/doc search via `docs_search`: `cratesio`/`npm`/`mdn` on by default, plus
  opt-in `rubygems`/`packagist`/`nuget`/`hex`/`aur`/`dockerhub`/`archlinux`; the
  kind aggregates across ecosystems), composite
  (`github`, `stackoverflow`), and bespoke (`grep_app`, `medium`, `searxng`). Each
  documented under `docs/providers/`.
- **Self-hosted forges:** register private GitLab/Gitea hosts under `[forges]`;
  each becomes a keyless `code_<id>` provider.
- **SearXNG provider** (web + code) against a self-hosted instance's JSON API.
- **Search strategies** `fallback` and `aggregate` (concurrent meta-search) with
  a **composite** ranker by default — weighted Reciprocal Rank Fusion (k=60) ×
  cross-engine consensus × lexical relevance × authority, then MMR domain
  diversification (tunable via `[search.engine_weights]`/`trusted_domains`) — plus
  `reciprocal`, `borda`, `breadth`, and `interleave`, all overridable **per kind**
  via `[search.web]/[search.code]/[search.qa]`.
- **Model-controlled rendering:** any HTML-scraping provider can run through a
  shared, persistent headless Chrome via a per-call `render` flag; scrape is the
  default.
- **In-memory caching** (`[cache]`, on by default, 300s TTL): search results
  keyed by the normalized query, plus retrieval-tool output (`fetch_page`,
  `render_page`, `fetch_repo_file`, `wayback_fetch`, `qa_stackoverflow_answers`)
  in a separate store keyed by the request. Only non-empty results are cached.
- **Hivemind** (`[network]`, opt-in/off by default): peer-to-peer consult of
  other instances' caches before scraping, with static + mDNS discovery plus
  **gossip** (mesh grows from a seed), **bounded relay** across the graph
  (`relay_hops`), Bloom-filter digests, a hash-only wire protocol, and
  consensus/reputation anti-poisoning with optional reputation **persistence**
  (`/hive/digest`, `/hive/query`). The `hive_status` tool shows the mesh graph.
  See `docs/hivemind.md`.
- **Configurable HTTP timeout** with a single short-backoff retry on the
  engine/forge paths.
- **Optional bearer-token auth** on `/mcp` (`auth_token` / `LODESTONE_AUTH_TOKEN`,
  constant-time compare); `/health` stays open.
- **Layered configuration:** built-in defaults < `config/**.toml` (deep-merged) <
  `lodestone.toml` < environment variables. Granular, documented per-provider
  and per-feature config files; preset examples under `examples/`.
- **Docker image** bundling Chromium; **CI** (fmt/clippy/build/test) plus a
  path-gated Docker build + `/health` smoke test; release workflow on `v*` tags.
- **Optional credentials**, all keyless-by-default: GitHub token (authenticated
  code-search API) and StackExchange API key (raises quota), read from config or
  env, never logged or committed.

[Unreleased]: https://github.com/elyerinfox/lodestone-mcp/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/elyerinfox/lodestone-mcp/releases/tag/v0.1.0
