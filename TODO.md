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

- [x] **Framework documentation providers.** Done: `src/providers/docsite/` — a
  spec-driven family (like `forge`) doing a keyless, site-scoped web search of a
  framework's docs domain (DuckDuckGo → Mojeek, render-aware). Built-ins for PHP,
  Laravel, Vue, React, Svelte (default-on) plus a dozen more opt-in; custom hosts
  via `[docsites.<id>]`. Each gets a `docs_<id>` tool and joins `docs_search`
  (which gained a `render` flag). Docs: `docs/providers/frameworks.md`.

- [x] **Translation tools.** Done: `src/translate.rs` — keyless Google Translate
  (`translate_a/single`), exposed as the `translate` and `detect_language` tools
  (standalone, like the `datetime` family; cached). No API key.

- [x] **Container & cloud-native tools.** Done: `src/oci.rs` (Docker Hub JSON API
  for `docker_search`/`docker_image`/`docker_tags`; generic OCI Distribution access
  with the anonymous bearer-token flow for `oci_tags`/`oci_manifest` across Docker
  Hub/GHCR/Quay/self-hosted) and `src/artifacthub.rs` (`artifacthub_search` over
  Helm/Operators/krew/policies/Tekton). Plus `docker`/`kubernetes`/`helm` doc sites
  in the docsite family. All keyless. Docs: `docs/containers.md`.
- [x] **Local Docker daemon control.** Done: `src/docker.rs` talks to the daemon
  directly via the Engine API over the platform socket (bollard; Windows named pipe
  / unix socket; honors `DOCKER_HOST`) — no `docker` CLI. Granular per-action tools,
  gated by `[docker]` (on by default; `allow_destructive` off by default hides
  `docker_stop`/`docker_remove`). Docs: `docs/containers.md`.

- [x] **Kubernetes cluster interaction.** Done: `src/k8s.rs` — direct API (kube-rs;
  reads kubeconfig / `$KUBECONFIG` / in-cluster), granular per-action tools gated by
  `[kubernetes]` (on by default; destructive opt-in): `k8s_contexts`, `k8s_get`,
  `k8s_describe`, `k8s_logs`, `k8s_apply` (kubefiles, server-side apply), `k8s_scale`,
  `k8s_delete`. Kinds resolved via API discovery. Docs: `docs/containers.md`.
  - **Deferred within this:** `k8s_exec` (SPDY/ws), `rollout restart`, and Helm
    release *mutation* (would reimplement Helm — out of scope for direct-API; Helm
    docs + Artifact Hub search already cover discovery).

- [x] **Docker daemon — more actions.** Done: `docker_build` (tars a local context
  directory and streams the daemon build log), `docker_exec` (run a command in a
  running container), and `docker_rmi` (image removal). `docker_exec`/`docker_rmi`
  are destructive → routed through the confirmation guard (golden rule 8);
  `docker_build` is write-class. Added `tar`/`bytes` deps; docs + Tools list updated.

- [x] **Database client skills (Redis, MySQL, PostgreSQL).** Done:
  `src/skills/databases.rs` with `db_list`, `db_query` (PostgreSQL/MySQL via `sqlx`),
  and `redis_command` (via the `redis` crate). Connections from `[databases.<id>]`
  (kind + URL + per-instance `allow_destructive`); URLs are treated as secrets (never
  listed/logged). **Off by default** — the tools appear only when ≥1 instance is
  configured. Reads run freely; non-SELECT SQL and non-read Redis commands are
  destructive → routed through the confirmation `guard` (golden rule 8), pre-authorized
  by `allow_destructive`. Connect failures surface a clear, contextual error. Rows
  rendered with safe per-column type probing (capped at 200 rows). Docs +
  `config/14-databases.toml` added.

- [x] **System information skill.** Done: `src/skills/sysinfo.rs` exposes `system_info`
  (host/OS/kernel/uptime, CPU model+cores+usage, memory/swap), `system_disks`
  (mount/fs/total/used/free), and `system_gpu` (NVIDIA via NVML: name/memory/
  utilization/temperature). Cross-platform via the `sysinfo` crate (Linux /proc+/sys,
  Windows OS APIs); GPU uses `nvml-wrapper` loaded at runtime — absent driver/library
  yields a clear message (dependency safeguard), not a failure. Gated by `[sysinfo]`
  (read-only, on by default); blocking work runs on `spawn_blocking`. Docs + config
  (`config/13-sysinfo.toml`) added.

- [ ] **SDR skill (RTL-SDR / HackRF).** Not supported yet. Would wrap `rtl-sdr` /
  `hackrf` (or `soapysdr`) to scan/sample radio — heavy native deps + hardware +
  drivers, off by default, side-effecting (tuning) → guarded. Likely shell out to
  `rtl_power`/`rtl_fm`/`hackrf_transfer` first (dependency safeguard when absent)
  rather than linking the C libs. Scope: `sdr_devices`, `sdr_scan` (power spectrum).

- [ ] **Background tasks & alerts.** A scheduler so the server can run periodic jobs
  (e.g. poll a feed, watch a container/quote/satellite pass) and surface alerts.
  MCP is request/response, so delivery is the hard part — options: MCP logging/
  notifications to the client, or a results buffer the model polls via a tool
  (`tasks_list`/`task_result`). The hivemind already runs a background sync loop to
  build on. Define create/list/cancel tools + a `[tasks]` config (off by default).

- [ ] **FFmpeg conversion skill.** There's an `ffmpeg` *docs* provider (`docs_ffmpeg`)
  but no conversion tool. Add `ffmpeg_convert` (input path, output path, optional
  args/codec/format) that shells out to the local `ffmpeg` binary — off by default
  (`[ffmpeg]`), paths confined to `[filesystem].roots`, output writes go through the
  confirmation guard, and a clear "ffmpeg not on PATH" safeguard. Maybe `ffmpeg_probe`
  (ffprobe metadata, read-only).

- [ ] **Time-series forecasting skill.** Forecast a numeric series (trend/seasonality).
  Prophet/SARIMAX are Python/statsmodels-heavy; in pure Rust the `augurs` crate (MSTL/
  ETS/AR) or a hand-rolled Holt-Winters + simple ARIMA is the practical path. Tool:
  `forecast` (values + horizon → point forecast + interval). Pure compute, local.
  Pulling real Prophet/SARIMAX would mean an embedded Python or a sidecar — out of
  scope for the single-binary model; document the chosen approximation.

- [ ] **News feed skill.** Subscribe-style RSS/Atom news by topic/source (generalizes
  the existing Medium RSS provider). Tool: `news_feed` (url or known source + topic →
  recent headlines/links/dates). Keyless RSS/Atom parse (`roxmltree`), cached.

- [ ] **Spreadsheet skill.** Read/edit tabular data. CSV is built-in (Rust `csv`);
  XLSX via `calamine` (read) + `rust_xlsxwriter` (write). Tools: `sheet_read`
  (range/sheet → rows), `sheet_query` (filter/select), `sheet_write` (cells/append).
  File writes are confined like the filesystem skill (`[filesystem].roots`) and go
  through the confirmation guard; off by default or behind `[filesystem]`.

- [x] **Serial device skill.** Done: `src/skills/serial.rs` — `serial_ports`,
  `serial_send` (guarded write), `serial_read` (timed read → text + hex) via the
  `serialport` crate. Off by default (`[serial]`); blocking I/O on `spawn_blocking`.

- [x] **Printer skill.** Done: `src/skills/printer.rs` — `printer_list` +
  `printer_print` (guarded), shelling to CUPS `lp`/`lpstat` (Unix) or PowerShell
  `Get-Printer`/`Out-Printer` (Windows). Off by default (`[printer]`); "no print
  system" safeguard.

- [x] **Stock market quote skill (NYSE/NASDAQ).** Done: `src/skills/stocks.rs` —
  `stock_quote` via the keyless Stooq CSV endpoint (delayed OHLC + volume; US tickers
  auto-suffixed `.us`, indices/forex pass through). Cached; documented as delayed
  reference data. (A keyed provider for richer/history data remains a future option.)

- [x] **NASA API skills.** Done: `src/skills/nasa.rs` — `nasa_apod`, `nasa_neo`,
  `nasa_mars_photos` against api.nasa.gov (keyless via `DEMO_KEY`; optional
  `[nasa].key`/`LODESTONE_NASA_KEY` raises the limit). Cached. (DONKI/Exoplanet and an
  ESA per-service skill remain future options — ESA has no single unified API.)

- [x] **Satellite trajectory skill.** Done: `src/skills/satellite.rs` — `sat_tle`
  (fetch a TLE from CelesTrak by NORAD id/name), `sat_position` (SGP4 → ground
  sub-point lat/lon/alt + speed), `sat_observe` (azimuth/elevation/range from an
  observer). TEME→ECEF via GMST, WGS-84 geodetic, topocentric SEZ look-angles;
  unit-tested (geodetic round-trip, GMST vs J2000, ISS LEO sanity).

- [x] **Wave/frequency calculation.** Done (math helper): `wave_frequency` converts
  between frequency, wavelength, and period via v = f·λ (speed defaults to c; set
  ~343 for sound in air). SI-scaled output.

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

- [x] **StackExchange answers via render.** Done: `qa_stackoverflow_answers` gained a
  `render` flag — when set (and the site is `stackoverflow`), it scrapes the question
  page via the shared headless browser instead of the API, parsing the question body
  + top answers (score/accepted/code) from `.s-prose`/`.answer[data-score]`. Falls
  back to the API for other sites; cache key includes the render mode. Parser unit-tested.

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
  - **Done (Redis backend):** `[cache].backend = "redis"` + `redis_url`
    (`LODESTONE_CACHE_BACKEND`/`LODESTONE_CACHE_REDIS_URL`) selects a shared Redis
    store implementing the same `get`/`put`/`keys` contract (search + retrieval
    caches namespaced by key prefix), so multiple instances share results. The sync
    cache API bridges Redis's async client via `block_in_place`; on connect failure
    it falls back to the in-memory backend.

- [x] **Cache audit + file store + cache-management skills.**
  - **Done (1) Audit:** every networked lookup now caches. `arxiv_search`/`arxiv_get`,
    `hf_search`/`hf_model`, and `kernel_releases` were missing caching and now route
    through the retrieval cache; search/docs/rfc/standards/wikipedia/oci/dockerhub/
    artifacthub/github/translate and the retrieval tools already did. Deliberately
    *not* cached: system-specific/sensitive families (docker/k8s/fs/shell/git/
    sysinfo/databases) and purely-local tools (datetime/data/regex/math/units).
  - **Done (2) File store:** `src/store.rs` — a key-addressed on-disk byte store
    (`<hash>.data` + `<hash>.key` sidecar) with TTL + byte-budget retention enforced
    on write. Off by default.
  - **Done (3) Cache-management skills:** `store_fetch` / `store_get` / `store_list` /
    `store_purge` (gated by `[store]`), plus `cache_status` (always on) reporting the
    in-memory search + retrieval caches and the file store.
  - **Done (4) Config:** `[store]` (`enabled`/`dir`/`ttl_secs`/`max_bytes` +
    `LODESTONE_STORE_*`); `[cache]` already had retention knobs (+ the Redis backend).
    Documented in `config/15-store.toml`.
  - **Done (5) Hivemind sharing of the file store:** the digest Bloom now advertises
    file-store entry hashes alongside search-cache keys, and a new `/hive/blob`
    endpoint serves a cached file's raw bytes by hash. `read_pdf` and `store_fetch`
    go through `Lodestone::fetch_bytes_shared` (local store → a hive peer that has it
    → the source, caching the result), so a PDF/file one node fetched (arXiv, IETF, …)
    is served from the mesh instead of every node re-hitting the rate-limited source.
    Only hashes cross the wire; blob serving honors `[network].token`; non-hex keys
    are rejected (no path traversal); no relay/consensus for blobs (the consumer
    re-fetches from the source if a peer's bytes are unusable). The retrieval text
    cache is intentionally *not* shared yet — file bytes were the rate-limit win.

- [x] **Headless-browser page pool.** Done: `ChromiumRenderer` no longer serializes
  renders behind a single mutex — the browser lives behind an `RwLock` (read to use,
  write only to launch/relaunch) and renders run as **concurrent pages** on it,
  bounded by a `Semaphore` sized by `[google].render_concurrency` (default 4,
  `LODESTONE_RENDER_CONCURRENCY`). Relaunch-on-crash is single-flighted via
  `Arc::ptr_eq` so concurrent renders that hit a dead browser trigger one relaunch.

- [x] **Aggregate request economy.** Done: `[search].max_concurrency` (default 8,
  `LODESTONE_SEARCH_MAX_CONCURRENCY`, 0 = unlimited) bounds how many providers run
  at once in aggregate mode via a `tokio::Semaphore` in `search_aggregate` — so a
  wide `docs` fan-out (each doc site hitting DuckDuckGo) queues instead of bursting
  past engine rate limits. (Query coalescing across forges was considered but not
  needed once concurrency is bounded; left as a possible future optimization.)

- [x] **DuckDuckGo endpoint rotation/backoff.** Done: `EngineSpec` gained an
  `alts` list of interchangeable endpoints; the DuckDuckGo spec declares the
  `lite.duckduckgo.com/lite/` primary plus the `html.duckduckgo.com/html/` mirror
  (with a custom parser that decodes the `/l/?uddg=` redirect links). `search_raw`
  rotates the starting endpoint (round-robin, to spread IP load) and falls through
  to the next on error/empty with a growing backoff, keeping a single in-place retry
  per endpoint. Mojeek/Google declare `alts: &[]` (unchanged behavior).

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
  - **Done since:** a Redis-backed *shared* cache (multiple nodes behind one store)
    now exists via `[cache].backend = "redis"` (see the cache item above).

## Docs & release

- [~] **CHANGELOG.md and a tagged release.**
  - **Done:** `CHANGELOG.md` started (Keep a Changelog) with the 0.1.0 entry.
  - **Remaining (needs maintainer sign-off):** cut `v0.1.0`
    (`git tag v0.1.0 && git push origin v0.1.0`) to exercise the binary + Docker
    release pipeline. Left untagged deliberately — tagging publishes artifacts.
