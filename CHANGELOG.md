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
- **Hugging Face skills** (keyless): `hf_search` (models or datasets) and `hf_model`
  (model metadata: downloads, likes, task, library, license, tags).
- **Standards lookup** (keyless): `standards_search` finds published standards
  (IEEE, SAE, NIST, ISO, ANSI, IEC, …) via the Crossref API — title, publisher,
  type, year, DOI, and a doi.org link (metadata; IEEE/SAE are paywalled, NIST is
  free). Plus `ieee`/`sae`/`nist` doc-site providers (`docs_ieee`/`docs_sae`/
  `docs_nist`) for the publishers' own pages.
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
  code-search API), StackExchange API key (raises quota), and the keyed
  `apiengine` web providers `brave` (Brave Search API) and `google_cse` (Google
  Programmable Search) — each off unless its key is set. Read from config or env,
  never logged or committed.

[Unreleased]: https://github.com/elyerinfox/lodestone-mcp/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/elyerinfox/lodestone-mcp/releases/tag/v0.1.0
