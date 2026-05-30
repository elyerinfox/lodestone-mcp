# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Signal-processing skills** (off by default, `[signal]`): `signal_fft`,
  `signal_dominant_frequencies`, `signal_rms`, `signal_window` (Hann / Hamming
  / Blackman / rectangular). Pure compute via `rustfft` (runtime SIMD).
- **WAV file skills** (off by default, `[wave]`): `wave_info`, `wave_samples`
  via `hound`. Pair with the signal skills to FFT decoded audio.
- **Binary analysis skills** (off by default, `[binary]`): `binary_info` (ELF/
  PE/Mach-O via `object`), `binary_strings` (printable-string extraction),
  `binary_entropy` (Shannon entropy per block — spot packed/encrypted
  regions), `binary_hexdump`. Read-only.
- **Pcap reader skills** (off by default, `[pcap]`): `pcap_info`,
  `pcap_packets` via the pure-Rust `pcap-file` crate (no native libpcap).
- **x86/x64 disassembly skills** (off by default, `[disasm]`):
  `disasm_x86_hex`, `disasm_x86_file` via `iced-x86` (NASM-flavored output).
- **Jupyter notebook skills** (off by default, `[notebook]`): `notebook_info`,
  `notebook_cells`. Read-only `.ipynb` parser.
- **Python runner skill** (off by default, `[python]`): `python_run`
  subprocess to system interpreter; every call confirms first (guarded).
- **Linux systemd skills** (off by default, `[systemd]`): `systemd_list`,
  `systemd_status`, `systemd_logs` (read-only), plus guarded
  `systemd_start` / `stop` / `restart`.

- **Persistent memory & solution-history skills** (off by default, `[memory]`).
  Two related on-disk tool families share one local JSONL store under
  `[memory].dir` (default `.lodestone-memory/`):
  - **`memory_*`** (`save`/`get`/`list`/`search`/`forget`) — a simple key→value
    store the model can write to remember anything across sessions and restarts.
    Optional `scope` namespaces and `tags`.
  - **`solution_*`** (`record`/`find`/`show`/`list`/`update`/`forget`) — a
    record of proposed solutions to past problems, with full revision history.
    `solution_find` surfaces matching prior entries as **advisory suggestions
    only** — never prescriptive — ranking by *exact canonical key* > *exact
    concept tokens* > *fuzzy Jaccard concept-overlap* > *substring*, plus a
    boost for shared `tags`. `solution_update` appends a new revision (prior
    revisions stay queryable via `solution_show`).
  - **Typed relation graph** (`solution_link` / `solution_unlink` /
    `solution_graph` / `solution_related`) — declare auto-reciprocal edges
    between solutions (`supersedes`↔`superseded-by`,
    `depends-on`↔`dependency-of`, plus symmetric `related-to` / `see-also` /
    `alternative-to` / any free-form kind). `solution_graph` walks the explicit
    subgraph around an id (BFS, default 2 hops, max 5); `solution_related`
    returns a combined ranking that also weighs shared tags and concept-token
    overlap. `solution_forget` cleans dangling incoming edges.

  The journals are append-only; on startup the server replays them and
  atomically rewrites each file with the current snapshot, so size stays
  bounded. Entries are **local only** — never advertised in the constellation
  digest. `*_forget` are destructive (guarded; `[memory].allow_destructive` pre-
  authorizes). Reuses the canonical/concept-token normalization the search
  cache uses, so a reworded later question still finds the prior entry.
- **Single-token synonym fold** in `canonical_query` / `concept_tokens`
  (`src/provider.rs`): a small alias table (`k8s`↔`kubernetes`, `ssl`↔`tls`,
  `gh`↔`github`, `js`↔`javascript`, `ts`↔`typescript`, `py`↔`python`,
  `rb`↔`ruby`, `go`↔`golang`, `sh`↔`shell`, `db`↔`database`,
  `config`/`conf`/`setup`↔`configure`) is applied before stop-wording. Affects
  both the search cache and the memory/solution recall — a query for
  `"k8s deploy"` now reuses a cached/recorded `"kubernetes deploy"` result.
- **Scientific formula library, organized by field.** A shared formula-registry
  engine (`src/skills/formula.rs`) backs per-field named-formula tools: **physics**
  (`physics_formula`/`physics_formula_list` — ~70 formulas across mechanics,
  gravitation, EM, thermodynamics, waves/optics, relativity, atomic/nuclear, fluids —
  plus `physical_constant`), **geometry** (`geometry_formula`), **trigonometry**
  (`trig_formula`), and **algebra/combinatorics** (`algebra_formula`). Call
  `<field>_formula` with a `{var: value}` map (SI units, angles in degrees) and
  `<field>_formula_list` to discover ids.
- **Background-tasks skill** (`task_run`/`task_list`/`task_status`/`task_result`/
  `task_cancel`, off by default `[tasks]`): run long work (currently a search) off the
  request path and poll for results — model-polled, so it works on any client
  including LM Studio. Bounded job table with eviction; cancellable.
- **Open-access skills** (`unpaywall_lookup`, `openalex_search`, `openalex_work`):
  find *legal* full-text copies of papers — Unpaywall (best OA copy by DOI) and
  OpenAlex (search/fetch works with OA PDF links) — to feed `read_pdf`. Keyless;
  Unpaywall needs a contact email (`LODESTONE_CONTACT_EMAIL`). Surfaces only
  legitimately open-access copies (no paywall circumvention).
- **PubMed + NCBI skills** (`pubmed_search`, `pubmed_summary`, `ncbi_search`,
  `ncbi_summary`): query NCBI via E-utilities (esearch/esummary/efetch) — the single
  API behind ncbi.nlm.nih.gov. PubMed tools cover the biomedical literature
  (abstracts, DOI); the generic `ncbi_*` tools reach **any** Entrez database via a
  `db` param (pmc, gene, protein, nucleotide, snp, clinvar, taxonomy, books, mesh, …).
  Keyless (optional `LODESTONE_NCBI_API_KEY` raises the rate limit); cached.
- **Galaxy** (optional, off by default): links constellations across networks. The
  **broker** is a *separate binary*, `lodestone-galaxy` — a rendezvous directory of
  `{ constellation → public ingress endpoint(s) }`, configured by env
  (`LODESTONE_GALAXY_BIND`/`TOKEN`/`TTL_SECS`). It is deliberately *not* a proxy:
  constellations fetch the directory and then talk directly over `/constellation/*`.
  The main `lodestone-mcp` app gains a **participation** side (`[galaxy].servers` +
  `ingress`): register this constellation and add other constellations as peers.
  Supports multiple ingress endpoints (distributed inbound) and inherent multi-egress;
  a node joins its own constellation first (warm-up) before reaching out. Broker
  endpoints: `POST /galaxy/register` / `…/heartbeat`, `GET /galaxy/directory`.
- **SDR skill** (`sdr_devices`, `sdr_scan`): list software-defined radios and sweep
  the RF spectrum by shelling out to `rtl_test`/`hackrf_info`/`rtl_power`. Off by
  default (`[sdr]`); **receive-only** (no transmit), with hardware/tool-absent
  safeguards.
- **Spreadsheet skill** (`sheet_read`, `sheet_query`, `sheet_write`): read/filter/write
  CSV/TSV and XLSX/XLS/ODS. Off by default (`[spreadsheet]`); paths confined to
  `[filesystem].roots`, writes routed through the confirmation guard. CSV via `csv`,
  XLSX reads via `calamine`, XLSX writes via `rust_xlsxwriter`.
- **FFmpeg skill** (`ffmpeg_probe`, `ffmpeg_convert`): probe and convert local media
  by shelling out to a system FFmpeg. Off by default (`[ffmpeg]`); paths confined to
  `[filesystem].roots`, conversions routed through the confirmation guard, with a
  clear "not on PATH" message when FFmpeg is missing.
- **Forecasting skills** — one tool per method, no hidden auto-selection:
  `forecast_holt_linear` (level + trend) and `forecast_holt_winters` (level + trend +
  additive season, needs a `season_length` and ≥2 full seasons). Smoothing constants
  (`alpha`/`beta`/`gamma`) can be pinned per call or, if omitted, are grid-searched on
  in-sample error; both return an approximate interval. A pragmatic single-binary
  stand-in for Prophet/SARIMAX (no Python, no network).
- **News-feed skill** (`news_feed`): fetch recent items (title/link/date/summary)
  from any keyless RSS 2.0 or Atom feed — a URL or a built-in shorthand
  (`hackernews`, `bbc`, `theverge`, `arstechnica`, `lobsters`, `lwn`). Read-only,
  cached; generalizes the Medium tag-RSS provider.
- **Yahoo Finance skill** (`yahoo_quote`, `yahoo_history`, `yahoo_search`): keyless,
  richer market data than the Stooq `stock_quote` — a full quote (change/%, day &
  52-week range, exchange, currency), OHLC history over a chosen range/interval, and
  symbol search. Uses Yahoo's public JSON endpoints (no key, no crumb). Gated by the
  existing `[stocks]` toggle.
- **Search circuit breaker** (`[search].breaker_threshold` / `breaker_cooldown_secs`):
  after N consecutive provider failures the source is skipped for a cooldown so it
  fails fast instead of re-waiting the deadline each call.
- **Fuzzy / concept query matching** (`[search].fuzzy_match`, off by default):
  searches are optionally also keyed by an order-independent, stemmed concept
  signature, so a reworded-but-equivalent query reuses a cached — or, over the
  constellation, a peer's (consensus-gated) — result on an exact-key miss.

### Changed

- **One tool per method (golden rule 9).** New invariant: a tool must not silently
  pick between distinct methodologies via an optional arg or heuristic — the method
  goes in the tool name so the model chooses it. Applied by splitting `hf_search`
  (a `kind` flag) into `hf_model_search` + `hf_dataset_search`. (The `forecast`
  split above is the same principle.) Targets addressed by an explicit user-supplied
  id/URL (e.g. `db_query` inferring Postgres/MySQL from the connection scheme) are
  *not* hidden selection and stay as-is.
- **Databases are now ad-hoc (no preconfiguration).** Dropped the stored
  `[databases.<id>]` instances and `db_list`; `db_query`/`redis_command` take a
  `connection` URL passed in the call (the credentials the user hands the model),
  with the engine inferred from the scheme. Gated by a simple `[databases].enabled`
  toggle; writes still confirm at call time (`[databases].allow_destructive`
  pre-authorizes), and URLs are never logged (summaries show only scheme + host).
- **`shell_run` now confirms at call time.** Because a shell command is arbitrary
  code, every `shell_run` is treated as destructive and routed through the
  confirmation guard (golden rule 8): the first call returns a one-time token and runs
  nothing; call again with `confirm=<token>` (or `confirm` + `trust=true` to whitelist
  that exact command). `[shell].allow_destructive` pre-authorizes. (Still off by
  default behind `[shell].enabled`.)
- **Split the `math` module by field** (breaking tool renames): `math_eval` →
  `arithmetic_eval` (new `arithmetic` module), `math_solve` → `algebra_solve` (new
  `algebra` module). `geo_distance`/`geo_azimuth` moved to `geometry` and
  `wave_frequency` to `physics` (tool names unchanged). The old `math` module is gone.
- **Multi-route egress for blocked providers** (`[search].proxy`,
  `[search].render_fallback`, both off by default): when a provider returns nothing
  or fails, it's retried over independent routes — direct → proxy (a different egress
  IP, e.g. a local `arti` SOCKS port; needs the new reqwest `socks` feature) → the
  headless browser — and the first route with results wins. Each route gets the
  per-provider deadline; the breaker counts a provider reachable if any route works.
- **Shared, convergent constellation id** (`[network].id`): member nodes share one
  constellation id (distinct from `node_id`); unset = random, and nodes that reach
  each other converge to the smallest id, so multi-node constellations register as a
  single galaxy entry and co-located meshes **merge**. The galaxy client registers
  under this id (unless `[galaxy].id` overrides). Galaxy participation is explicitly
  bidirectional — registering `ingress` allows traffic in, pulling the directory
  reaches out.
- **Constellation can listen on its own port** (`[network].bind`): when set, the
  `/constellation/*` endpoints serve on a separate listener so you can forward *only*
  that port (e.g. as a galaxy ingress) without exposing the `/mcp` server. Empty
  (default) keeps them merged on the main bind. Peers advertise this port.
- **Renamed the "hivemind" to the "constellation"** throughout (module
  `src/constellation`, the `constellation_status`/`constellation_peers`/
  `constellation_seeds` tools, the `/constellation/*` peer endpoints, and all docs).
  Behavior is unchanged; `[network]` config keys keep their names. A future
  cross-network linking layer that pairs multiple constellations is termed a
  **galaxy** (planned — see `docs/constellation.md`).
- **Per-provider search deadline** (`[search].provider_timeout_secs`, default 10):
  an unresponsive provider is dropped instead of stalling the whole search — the
  other engines still return in aggregate, and the chain moves on in fallback.
- **Query keys are canonicalized** (case/punctuation/stop-words/whitespace folded,
  word order preserved), so trivially-reworded queries share a cache/constellation key and
  hit each other's results.
- **Docs:** `docs/tools.md` regrouped strictly by purpose (finance/markets split out
  from space/astronomy); README gains a constellation "be a good neighbor" section.

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
- **Providers** across six families: engine (`duckduckgo`, `mojeek`, `google`),
  forge (`gitlab`, `codeberg`, `gitea`), registry (the `docs` kind, keyless JSON
  package/doc search via `docs_search`: `cratesio`/`npm`/`mdn` on by default, plus
  opt-in `rubygems`/`packagist`/`nuget`/`hex`/`aur`/`dockerhub`/`archlinux`; the
  kind aggregates across ecosystems), docsite (framework documentation), composite
  (`github`, `stackoverflow`), and bespoke (`grep_app`, `medium`, `searxng`). Each
  documented under `docs/providers/`.
- **Framework documentation providers** (docsite family, `docs` kind): keyless,
  site-scoped web search of a framework's docs (DuckDuckGo → Mojeek, render-aware),
  one `DocSiteProvider` per host. `php`/`laravel`/`vue`/`react`/`svelte` on by
  default; `angular`/`nextjs`/`nuxt`/`django`/`flask`/`fastapi`/`rails`/`spring`/
  `tailwind`/`express`/`symfony`/`astro`/`solid` opt-in. Register custom hosts via
  `[docsites.<id>] domain = "…"`. Each gets a `docs_<id>` tool and joins
  `docs_search`. `docs_search` gained a `render` flag for the SPA doc sites.
- **Translation tools** (Google Translate, keyless — no API key): `translate`
  (translate text to an ISO-639 target; auto-detects the source) and
  `detect_language` (report a text's language). Results are cached.
- **IETF RFC skills** (keyless): `rfc_get` fetches an RFC's full text by number
  directly from the RFC Editor; `rfc_search` finds RFCs by title via the IETF
  Datatracker.
- **Wikipedia skills** (keyless): `wikipedia_search` (MediaWiki full-text search)
  and `wikipedia_summary` (lead extract, or the full plain-text article with
  `full=true`); language is configurable (`lang`, default `en`).
- **kernel.org skill** (keyless): `kernel_releases` lists the current Linux kernel
  releases (mainline/stable/longterm, dates, EOL) from kernel.org's `releases.json`.
  Plus a `kernel` doc site (`docs_kernel`) for the kernel documentation.
- **arXiv skills** (keyless): `arxiv_search` (search papers) and `arxiv_get` (one
  paper's metadata + abstract). Each result includes the free PDF URL, so `read_pdf`
  retrieves the full text. Atom XML parsed with `roxmltree`.
- **Hugging Face skills** (keyless): `hf_model_search` and `hf_dataset_search` (each
  searches one corpus — no hidden mode flag) and `hf_model` (model metadata:
  downloads, likes, task, library, license, tags).
- **Standards lookup** (keyless): `standards_search` finds published standards
  (IEEE, SAE, NIST, ISO, ANSI, IEC, …) via the Crossref API — title, publisher,
  type, year, DOI, and a doi.org link (metadata; IEEE/SAE are paywalled, NIST is
  free). Plus `ieee`/`sae`/`nist` doc-site providers (`docs_ieee`/`docs_sae`/
  `docs_nist`) for the publishers' own pages.
- **Destructive-action confirmation** (client-agnostic, no MCP elicitation needed):
  `docker_stop`/`docker_remove`, `k8s_delete`, `fs_delete`/`fs_move`, and destructive
  `git_run` subcommands no longer act on the first call — they return a one-time
  `confirm` token describing the action and do nothing. Call again with
  `confirm=<token>` to perform it, or `confirm=<token>, trust=true` to also stop
  being asked for that action for the rest of the session. Destructive tools are now
  always exposed and gated at *call time* (rather than hidden); each family's
  `allow_destructive` pre-authorizes the action and skips the prompt. Tokens are
  single-use and expire after 5 minutes.
- **Space, markets & science skills** (keyless): `nasa_apod` / `nasa_neo` /
  `nasa_mars_photos` (api.nasa.gov, `DEMO_KEY` by default, optional `[nasa].key`);
  `stock_quote` (delayed quotes via Stooq CSV); `sat_tle` / `sat_position` /
  `sat_observe` (SGP4 orbital propagation — fetch a TLE from CelesTrak, then compute
  the ground sub-point or observer azimuth/elevation/range).
- **Device skills** (`[serial]`, `[printer]`, **off by default**): `serial_ports` /
  `serial_send` / `serial_read` (raw serial I/O via `serialport`) and `printer_list` /
  `printer_print` (CUPS `lp` / Windows spooler). Writes go through the confirmation
  guard; clear safeguards when the device/print system is absent.
- **System-information skills** (`[sysinfo]`, read-only, on by default): `system_info`
  (host/OS/kernel/uptime, CPU model+cores+usage, memory/swap), `system_disks`, and
  `system_gpu` (NVIDIA via NVML — clear message when the driver/library is absent).
  Cross-platform via `sysinfo` (Linux `/proc`+`/sys`, Windows OS APIs).
- **Database client skills** (`[databases.<id>]`, off until one is configured):
  `db_list`, `db_query` (PostgreSQL/MySQL via `sqlx`), and `redis_command`. Reads run
  freely; writes/DDL and write/admin Redis commands are destructive (confirmation
  guard; per-instance `allow_destructive` pre-authorizes). URLs are treated as secrets.
- **On-disk file store + cache management** (`[store]`, off by default): `store_fetch`
  (download + cache a URL's bytes), `store_get`, `store_list`, `store_purge`, with
  TTL + byte-budget retention; plus `cache_status` (always on) reporting the in-memory
  search/retrieval caches and the store. Every networked lookup now caches
  (arxiv/hf/kernel added).
- **Constellation file & retrieval sharing**: the digest advertises file-store entry
  hashes *and* retrieval-cache keys; `/constellation/blob` serves a cached file/page's bytes
  by hash. `read_pdf` and `store_fetch` resolve URLs as local store → a constellation peer →
  the source, so a PDF/file one node fetched (arXiv, IETF, …) is served from the mesh
  instead of every node re-hitting the rate-limited source. Only hashes cross the
  wire; token-gated.
  - **Anti-tampering**: a blob is trusted only when `>= [network].min_agreement` peers
    **corroborate** its content hash (`/constellation/blobinfo`), and the fetched bytes are
    **verified** against that hash before use (else fall back to source).
  - **Seed accounting**: per-blob served-vs-fetched byte ratio (BitTorrent-style),
    shown by the `constellation_seeds` tool and in `store_list`.
- **Constellation introspection + identity**: nodes now have a stable, machine-derived id
  (`machine-uid` + bind port); new `constellation_peers` (per-node hop distance + machine id)
  and `constellation_seeds` (seed ratios) tools join `constellation_status`.
- **Redis cache backend** (`[cache].backend = "redis"`): a shared store multiple
  instances point at, behind the same get/put contract (falls back to in-memory on
  connect failure).
- **More Docker daemon actions**: `docker_build` (tar a context), `docker_exec`, and
  `docker_rmi` (exec/rmi are destructive → confirmation guard).
- **StackExchange answers via render**: `qa_stackoverflow_answers` gained a `render`
  flag to scrape the question page (saves API quota; stackoverflow.com only).
- **Engine resilience & economy**: DuckDuckGo rotates between its `lite`/`html`
  endpoints with backoff; aggregate search is bounded by `[search].max_concurrency`
  (default 8); the headless browser renders concurrent pages bounded by
  `[google].render_concurrency` instead of serializing on one mutex.
- **Dependency safeguards:** skills that need an external binary/runtime now fail
  with a clear, actionable message when it's missing — `git_run`/`shell_run` report
  "not found on PATH (is it installed?)", and the headless-browser paths
  (`render_page`/`webpage_to_pdf`/`google`) explain that Chrome/Chromium is required
  (and how to point at it). Docker/Kubernetes already report connection failures.
- **Git CLI skill** (`git_run`, `[git]`, on by default): runs the local `git`
  binary in a repo (no shell); destructive subcommands (push/reset/clean/rebase/…)
  require `[git].allow_destructive`.
- **Shell execution** (`shell_run`, `[shell]`, **off by default** — arbitrary code
  execution). Allowlist mode runs only `[shell].allow` programs, executed directly
  without a shell (metacharacters inert); `allow_unrestricted` runs anything via the
  system shell. Per-command timeout (killed) and working directory.
- **Local filesystem skills** (`[filesystem]`, **off by default** — explicit grant
  required): `fs_read`, `fs_list`, `fs_stat`, `fs_find`, `fs_write`, `fs_edit`,
  `fs_mkdir`, plus destructive `fs_delete`/`fs_move` (only when `allow_destructive`).
  All paths are confined to `[filesystem].roots` (default: the working directory);
  `..` and symlink escapes are rejected.
- **More doc sites:** `ffmpeg` (ffmpeg.org), `nvidia` (docs.nvidia.com), `intel_arc`
  (intel.com), `tailwind` (tailwindcss.com), `bootstrap` (getbootstrap.com) — on by
  default → `docs_ffmpeg` / `docs_nvidia` / `docs_intel_arc` / `docs_tailwind` /
  `docs_bootstrap`.
- **Local utility skills** (no network): `json_query` / `json_format` /
  `yaml_to_json` / `json_to_yaml` (parse, search by JSON Pointer, convert, format);
  `regex_search` / `regex_replace` (Rust regex syntax); `math_eval` (arithmetic/
  scientific expressions) and `math_solve` (linear/quadratic equations in `x`);
  and `convert_units` (length/mass/volume/area/speed/time/data/temperature).
- **Container & cloud-native tools** (keyless): `docker_search` / `docker_image` /
  `docker_tags` (Docker Hub image search, metadata, and tags via the public JSON
  API); `oci_tags` / `oci_manifest` (list tags and inspect a manifest — platforms
  or layers/size — on **any** OCI registry: Docker Hub, GHCR, Quay, self-hosted,
  via the Distribution Spec's anonymous bearer-token flow); and `artifacthub_search`
  (Artifact Hub: Helm charts, Operators, krew plugins, policies, Tekton tasks, with
  an optional `kind` filter). The framework-docs family adds `docker`/`kubernetes`/
  `helm` doc sites (on by default). See `docs/containers.md`.
- **Local Docker daemon control** (`[docker]`, on by default) — talks to the daemon
  directly via the Engine API over the platform socket (Windows named pipe / unix
  socket; honors `DOCKER_HOST`), no `docker` CLI. Each action is its own gated tool:
  read/safe-write — `docker_ps`, `docker_images`, `docker_inspect`, `docker_logs`,
  `docker_info`, `docker_pull`, `docker_run`, `docker_start`; destructive
  (`docker_stop`, `docker_remove`) hidden unless `[docker].allow_destructive`.
- **Kubernetes cluster interaction** (`[kubernetes]`, on by default) — talks to the
  API server directly via kube-rs, reading your kubeconfig (default / `$KUBECONFIG`
  / configured path+context) or in-cluster credentials, no `kubectl`. Granular
  per-action tools: read/safe-write — `k8s_contexts`, `k8s_get`, `k8s_describe`,
  `k8s_logs`, `k8s_apply` (server-side apply of kubefiles), `k8s_scale`; destructive
  `k8s_delete` hidden unless `[kubernetes].allow_destructive`. `kind` accepts
  kubectl-style names via API discovery.
- **Self-hosted forges:** register private GitLab/Gitea hosts under `[forges]`;
  each becomes a keyless `code_<id>` provider.
- **SearXNG provider** (web + code) against a self-hosted instance's JSON API.
- **PDF tools** (local-only, no external service): `webpage_to_pdf` renders a page
  to a PDF via the headless browser; `read_pdf` extracts a PDF's text (URL or
  local path) with `pdf-extract`. `fetch_page` also auto-detects PDFs and extracts
  their text. Scanned/image-only PDFs (no text layer) return an error.
- **Date/time tools** — `datetime` (current local/UTC/Unix time, plus an optional
  IANA timezone), `date_diff` (difference between two dates: days/years and
  ago/from-now), and `time_convert` (convert a time to another IANA timezone).
  Helps the model anchor recency and do timezone math (chrono + chrono-tz).
- **GitHub tools** (keyless, optional `[github].token` to raise the rate limit):
  `github_releases` (release notes / changelogs), `github_user` (profile), and
  `github_repo` (repo metadata), all accepting `owner/repo` or a github.com URL.
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
- **Constellation** (`[network]`, opt-in/off by default): peer-to-peer consult of
  other instances' caches before scraping, with static + mDNS discovery plus
  **gossip** (mesh grows from a seed), **bounded relay** across the graph
  (`relay_hops`), Bloom-filter digests, a hash-only wire protocol, and
  consensus/reputation anti-poisoning with optional reputation **persistence**
  (`/constellation/digest`, `/constellation/query`). The `constellation_status` tool shows the mesh graph.
  See `docs/constellation.md`.
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
  code-search API), StackExchange API key (raises quota), and the keyed
  `apiengine` web providers `brave` (Brave Search API) and `google_cse` (Google
  Programmable Search) — each off unless its key is set. Read from config or env,
  never logged or committed.

[Unreleased]: https://github.com/elyerinfox/lodestone-mcp/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/elyerinfox/lodestone-mcp/releases/tag/v0.1.0
