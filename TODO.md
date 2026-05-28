# Roadmap / TODO

Outstanding work and planned improvements for lodestone-mcp. Each item states
**what** to do, **why** it matters, and **how** to approach it (with the files
likely involved). Checked items are done; unchecked are open.

---

## Testing & CI

- [x] **Fixture-based parser tests.** Done: hermetic `#[cfg(test)]` tests with
  inline fixtures pin the parsers — the selector engine (DuckDuckGo/Mojeek shared
  path), grep.app JSON, GitHub code-search JSON, StackExchange search JSON +
  `parse_stat`, Medium RSS + `tag_slug`, SearXNG JSON, and the forge blob-URL
  parsers / `forge::repo_path`. (Chose inline fixtures over `tests/fixtures/`
  files since the parse fns are private module fns; integration tests can't reach
  them.) Not covered: Google's headless-only custom parser and the StackOverflow
  `/search` scrape parser (both render-only paths).

- [x] **Config-merge unit tests.** Done: `src/config.rs` tests cover
  `merge_tables` (nested key-by-key merge, scalar override, wholesale array
  replacement) and `Config` deserialization of a merged table with overlay
  precedence (plus serde-default fill-in).

- [x] **Build and smoke-test the Docker image in CI.** Done: a `docker` job in
  `.github/workflows/ci.yml` builds the image (no push) when
  `Dockerfile`/`Cargo.*`/`src/**` change (gated via `dorny/paths-filter`), then
  runs the container and polls `/health` to confirm it boots.

---

## Configuration

- [x] **Per-kind search strategy.** Done: `[search.web]/[search.code]/[search.qa]`
  override `strategy`/`ranking` (empty field = inherit the global `[search]`),
  resolved into a per-kind `KindPlan` threaded through `Registry::search`/
  `describe` in `src/provider.rs`.

- [x] **Configurable self-hosted forge instances.** Done: `[forges.<id>] kind =
  "gitlab"|"gitea", domain = "…"` (`config/04-forges.toml`) builds a
  `ForgeCodeProvider` via `forge::make_configured` (leaks id/domain/spec to
  `'static`, reusing the host-agnostic layout parsers). Activate by adding `<id>`
  to `[providers].code`; it gets a `code_<id>` tool automatically.

---

## Providers

- [x] **SearXNG provider.** Done: `src/providers/bespoke/searxng.rs` hits
  `{url}/search?format=json` for web+code (code is `site:`-scoped to
  `[code].sites`), parses the `results` array into `SearchResult`. Config
  `[searxng].url` (`LODESTONE_SEARXNG_URL`); disabled when empty. Docs +
  `config/providers/searxng.toml` + per-provider page added.

- [x] **Provider-level timeouts and limited retries.** Done: configurable
  `[search].timeout_secs` (`LODESTONE_SEARCH_TIMEOUT_SECS`) on the shared HTTP
  client, plus a single short-backoff retry in engine `search_raw` — which the
  forge `search` path inherits via its DuckDuckGo/Mojeek calls.

---

## Retrieval

- [x] **Multi-forge raw file fetch.** Done: `retrieve::resolve_raw_file` resolves
  GitHub (`/blob/`), GitLab (`/-/blob/` → `/-/raw/`), and Gitea/Codeberg
  (`/src/branch|commit|tag/` → `/raw/…`) URLs (and the GitHub `owner/repo/path`
  shorthand); the tool is now `fetch_repo_file`.

- [ ] **StackExchange answers via render.**
  - **Why:** `qa_stackoverflow_answers` always uses the API (quota); for parity
    with `qa_search`, allow `render=true` to scrape the question page.
  - **How:** Add a `render` arg to the tool and a scrape path in
    `src/providers/stackexchange.rs` / `src/retrieve.rs` reusing the shared
    renderer.

- [x] **PDF parsing (local) + page→PDF.** Done: `fetch_readable` detects PDFs
  (content-type / `.pdf` / `%PDF` magic) and extracts the text layer locally with
  `pdf-extract` (off the async runtime). New tools: `read_pdf` (URL or local path)
  and `webpage_to_pdf` (render a page to a local PDF via the headless browser). All
  local, no external service. Scanned/no-text-layer PDFs return a clear error.
  - **Deferred:** OCR for scanned PDFs (would imply a credentialed external
    service like Textract — out of scope for the local-only requirement).

---

## Performance & resilience

- [x] **Result cache (in-memory).** Done: `src/cache.rs` `TtlCache` (TTL +
  size-bounded), wired into `Registry::search`/`run_one` keyed by the normalized
  query; only non-empty results are stored, secrets never are. Config `[cache]`
  (`enabled`/`ttl_secs`/`max_entries`, `LODESTONE_CACHE_*`), on by default at
  300s. See `config/05-cache.toml`.
  - **Also done:** retrieval-tool output (`fetch_page`, `render_page`,
    `fetch_repo_file`, `wayback_fetch`, `qa_stackoverflow_answers`) is cached in a
    separate store keyed by the request (not shared into peer digests).
  - **Remaining:** an optional **Redis** backend (`[cache] backend = "redis",
    url = "redis://…"`) implementing the same get/put contract so multiple
    instances share results.

- [ ] **Headless-browser page pool.**
  - **Why:** The shared `ChromiumRenderer` serializes all renders behind one
    mutex; concurrent render-heavy use is bottlenecked.
  - **How:** Maintain a small pool of pages/contexts in `src/browser.rs`,
    bounded by config.

- [ ] **Aggregate request economy.**
  - **Why:** In `aggregate` mode each forge provider issues its own DuckDuckGo
    query, multiplying requests and tripping rate limits.
  - **How:** Cap concurrency, and/or coalesce site-scoped queries across forges
    into one engine call then split by domain.

- [ ] **DuckDuckGo endpoint rotation/backoff.**
  - **Why:** DuckDuckGo blocks aggressively by IP; a single endpoint with no
    backoff yields empty results under load.
  - **How:** Rotate `lite`/`html` endpoints and apply backoff in
    `src/providers/duckduckgo.rs`.

---

## Security & ops

- [x] **Optional bearer-token auth for the MCP endpoint.** Done: top-level
  `auth_token` (`LODESTONE_AUTH_TOKEN`); when set, an Axum `from_fn_with_state`
  middleware requires `Authorization: Bearer <token>` on `/mcp` (constant-time
  compare, 401 otherwise). `/health` stays open for probes.

- [x] **`/health` endpoint.** Done: a plain Axum `GET /health` returning `ok`
  alongside the `/mcp` service, for container/orchestrator liveness probes.

---

## Distributed / federation

- [x] **Peer-to-peer "hivemind" of instances with shared query knowledge.** Done
  (v1): `src/hive/` — opt-in `[network]` (off by default). Discovery via a static
  peer list **and** mDNS LAN (`_lodestone._tcp.local.`, runtime-disableable).
  Peers advertise a **Bloom filter** of cached query-key *hashes* (`GET
  /hive/digest`); `consult` asks matching peers (`POST /hive/query`, bounded +
  capped per peer) and **consensus** trusts a result only when `>= min_agreement`
  peers corroborate it (reputation-weighted, single-peer influence capped) —
  otherwise it falls back to a local search and learns from the peers. Only hashes
  cross the wire; responses carry only cached results (never secrets); `/hive`
  endpoints honor an optional `[network].token`. No relaying (no amplification).
  See `docs/hivemind.md` and `config/06-network.toml`.
  - **Also done:** gossip peer-exchange (digests carry known peers; mesh grows
    from a seed; dead peers pruned), **bounded relay** (`relay_hops`, ttl + seen
    loop-guard, each top-level peer still one consensus vote), reputation
    **persistence** (`state_file`), and a **`hive_status`** tool exposing the mesh
    graph (peers, reputation, reachability, edges).
  - **Deferred:** a Redis-backed *shared* cache (multiple nodes behind one store).

## Docs & release

- [~] **CHANGELOG.md and a tagged release.**
  - **Done:** `CHANGELOG.md` started (Keep a Changelog) with the 0.1.0 entry.
  - **Remaining (needs maintainer sign-off):** cut `v0.1.0`
    (`git tag v0.1.0 && git push origin v0.1.0`) to exercise the binary + Docker
    release pipeline. Left untagged deliberately — tagging publishes artifacts.
