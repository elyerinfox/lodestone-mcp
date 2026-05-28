# Roadmap / TODO

Outstanding work and planned improvements for lodestone-mcp. Each item states
**what** to do, **why** it matters, and **how** to approach it (with the files
likely involved). Checked items are done; unchecked are open.

---

## Testing & CI

- [ ] **Fixture-based parser tests.**
  - **Why:** Every provider parses scraped HTML/JSON whose markup changes without
    warning; today a broken selector fails silently at runtime with zero results.
    Tests pin parsing behavior and catch breakage in CI without hitting the network.
  - **How:** Save representative responses under `tests/fixtures/` (DuckDuckGo
    lite HTML, Mojeek results HTML, grep.app JSON, StackExchange search JSON,
    Medium tag RSS, StackOverflow `/search` HTML, GitHub code-search JSON). Add
    `#[cfg(test)]` unit tests in each provider that call the pure `parse(...)`
    function on a fixture and assert the extracted fields.
  - **Partly done:** the forge blob-URL parsers and `forge::repo_path` now have
    tests in `src/providers/forge/mod.rs`. Remaining: the HTML/JSON response
    fixtures for the engine/grep_app/stackexchange/medium/github parsers.

- [x] **Config-merge unit tests.** Done: `src/config.rs` tests cover
  `merge_tables` (nested key-by-key merge, scalar override, wholesale array
  replacement) and `Config` deserialization of a merged table with overlay
  precedence (plus serde-default fill-in).

- [ ] **Build and smoke-test the Docker image in CI.**
  - **Why:** The release workflow ships a Docker image that is never built on a
    normal push, so breakage is only discovered at tag time.
  - **How:** Add a CI job that runs `docker build .` (no push) on PRs touching
    `Dockerfile`/`Cargo.*`/`src/**`; optionally start the container and hit
    `/mcp` `initialize`.

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

- [ ] **SearXNG provider.**
  - **Why:** A user-hosted SearXNG instance gives high-quality, keyless,
    multi-engine results far beyond DuckDuckGo+Mojeek — the strongest keyless
    web/code option for users willing to run one.
  - **How:** New `web`/`code` provider hitting `{instance}/search?format=json`;
    config `[searxng].url`. Parse the JSON `results` array into `SearchResult`.

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

- [ ] **PDF parsing, with optional OCR for scanned PDFs.**
  - **Why:** Lots of docs/specs/papers are PDFs; `fetch_page` currently returns
    raw/garbled bytes for them, so the model can't read them.
  - **How:** Detect PDFs in `fetch_readable` (content-type, `.pdf`, `%PDF`
    magic) and extract the text layer with a Rust crate (`pdf-extract` / `lopdf`).
    For scanned PDFs with no text layer, optionally OCR via an operator-configured
    service — e.g. AWS **Textract** or any OCR endpoint — behind a config gate
    (`[pdf] ocr = "textract", …`). Keyless text extraction first; OCR strictly
    opt-in (it implies a credentialed external service).

---

## Performance & resilience

- [ ] **Result cache (in-memory, then optional Redis).**
  - **Why:** Repeated identical searches/fetches re-hit rate-limited engines and
    waste the StackExchange/GitHub quota; cached query results also make restarts
    and bursts cheap.
  - **How:** A cache trait wrapping engine calls, `fetch_readable`, and tool
    results, keyed by `(tool, normalized args)` with a TTL. Ship an in-memory
    backend first, then an optional **Redis** backend (config `[cache] backend =
    "redis", url = "redis://…", ttl_secs = …`) so multiple instances share cached
    "bits of information related to queries". Store small, serializable values
    (the normalized `SearchResult` list / extracted page text), never secrets.

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

- [ ] **Peer-to-peer "hivemind" of instances with shared query knowledge.**
  - **Why:** Independent instances re-do the same scraping and burn the same
    rate-limited engines. If a peer already searched something, a new instance
    should be able to consult the network before going out to the open web —
    spreading load, improving hit rates, and softening per-IP blocks. Local
    search must keep working with zero peers: the network is a *helper*, never a
    dependency.
  - **How:** (1) **Service discovery** — let instances find each other (static
    peer list in config, plus optional mDNS/LAN and a gossip seed). (2) **Shared
    digests** — each peer advertises what it has cached as a compact, privacy-
    preserving summary (e.g. a **Bloom filter** of normalized query keys, synced
    periodically), so a peer can cheaply test "might peer X have this?" without
    exchanging full query logs. (3) **Consult-then-fetch** — on a query, check
    peers whose Bloom filter matches, request the cached result (reuse the cache
    value format above), and fall back to a normal local search on miss/timeout/
    low-confidence. (4) **Rank peer results to prevent poisoning (required, not
    optional).** Never trust a peer's results blindly: corroborate across multiple
    peers, prefer results that the local engines also surface, and weight peers by
    a reputation score (decayed by disagreement/staleness). Treat peer data as
    *hints* that must survive the same dedup/ranking (incl. `breadth`/consensus)
    as first-party results, and cap any single peer's influence so one malicious
    or stale node can't dominate. Keep the network strictly opt-in
    (`[network] enabled = false` by default), bounded (timeouts, max peers), and
    careful never to share secrets or raw user inputs beyond hashed/Bloom keys.
    Likely its own module + a background sync task.

## Docs & release

- [ ] **CHANGELOG.md and a tagged release.**
  - **Why:** Users need to know what changed between versions; the release
    workflow already triggers on `v*` tags but none exist yet.
  - **How:** Start `CHANGELOG.md` (Keep a Changelog format); cut `v0.1.0`
    (`git tag v0.1.0 && git push origin v0.1.0`) to exercise the binary +
    Docker release pipeline.
