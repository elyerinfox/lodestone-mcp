# Tools reference

Every capability lodestone exposes, grouped by purpose. Tools come in two tiers:
**general** (aggregated, everyday) and **per-provider** (target one source). Every
tool is independently gateable via `[tools]` (see
[configuration.md](configuration.md#tools-skills)). `?` marks optional arguments.

## Search

Query all configured providers of a kind, combined per `[search].strategy`. All
accept `render` to route the underlying fetch through the headless browser.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `web_search` | `query`, `max_results?`, `render?` | General web search (DuckDuckGo, Mojeek, …). |
| `code_search` | `query`, `language?`, `max_results?`, `render?` | Source-code search across the configured forges (`[code].sites`). |
| `docs_search` | `query`, `max_results?`, `render?` | Package registries (crates.io, npm, MDN) **and** framework/tooling docs (PHP, Laravel, Vue, React, Svelte, Docker, Kubernetes, Helm, …). |
| `qa_search` | `query`, `site?`, `max_results?`, `render?` | Q&A providers (StackExchange network: StackOverflow, Server Fault, …). |

## Retrieve

Fetch one known thing.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `fetch_page` | `url`, `max_chars?` | Page → readable text over plain HTTP (the default reader). |
| `render_page` | `url`, `max_chars?` | Page → readable text via a headless browser (runs JS). |
| `webpage_to_pdf` | `url`, `path?` | Render a page to a local PDF (headless browser); returns the saved path. |
| `read_pdf` | `source`, `max_chars?` | Extract a PDF's text locally — `source` is a URL or local path. |
| `fetch_repo_file` | `target`, `start_line?`, `end_line?` | A file from GitHub/GitLab/Gitea — blob/raw URL, or GitHub `owner/repo/path` (a `#L10-L40` fragment works too). |
| `wayback_fetch` | `url`, `timestamp?`, `max_chars?` | Archived snapshot from the Wayback Machine. |
| `rfc_get` | `document`, `max_chars?` | An IETF RFC's full text by number (rfc-editor.org), keyless. |
| `rfc_search` | `query`, `max_results?` | Search RFCs by title via the IETF Datatracker, keyless. |
| `standards_search` | `query`, `publisher?`, `max_results?` | Search published standards (IEEE/SAE/NIST/ISO/ANSI/…) via Crossref. Metadata + DOI link; IEEE/SAE are paywalled (NIST is free — use `read_pdf`). |
| `arxiv_search` | `query`, `max_results?` | Search arXiv papers; returns title/authors/date/abstract + abs & free PDF URLs. |
| `arxiv_get` | `id` | One arXiv paper's metadata + full abstract + PDF URL (then `read_pdf` for full text). |
| `pubmed_search` | `query`, `max_results?` | Search PubMed (NCBI E-utilities, keyless): PMID, title, authors, journal, date, link. Supports field tags ([Title], [Author], …). |
| `pubmed_summary` | `pmid`, `max_chars?` | A PubMed paper's citation, DOI, link, and abstract text. |
| `hf_search` | `query`, `kind?`, `max_results?` | Search the Hugging Face Hub — models (default) or datasets. |
| `hf_model` | `model` | A Hugging Face model's metadata (downloads, likes, task, license, tags). |
| `wikipedia_search` | `query`, `lang?`, `max_results?` | Search Wikipedia (MediaWiki API); titles + snippets + URLs. |
| `wikipedia_summary` | `title`, `lang?`, `full?`, `max_chars?` | An article's lead summary, or the full plain-text article. |
| `kernel_releases` | — | Current Linux kernel releases (mainline/stable/longterm + dates/EOL) from kernel.org. |
| `news_feed` | `source`, `max_results?` | Recent items (title/link/date/summary) from an RSS or Atom feed — a URL or a shorthand (`hackernews`, `bbc`, `theverge`, `arstechnica`, `lobsters`, `lwn`). |

## GitHub (keyless; optional `[github].token` raises the rate limit)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `github_releases` | `repo`, `max_results?`, `include_prereleases?` | A repo's releases (newest first): tag, date, notes. |
| `github_user` | `user` | A GitHub user/org profile (bio, company, repos, followers). |
| `github_repo` | `repo` | Repo metadata (stars, language, topics, license, default branch, …). |

## Containers & cloud-native (keyless)

Full detail: [containers.md](containers.md).

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `docker_search` | `query`, `max_results?` | Search Docker Hub images (stars, pulls, official). |
| `docker_image` | `image` | A Docker Hub repository's metadata. |
| `docker_tags` | `image`, `max_results?` | A Docker Hub image's tags (size, date, arch). |
| `oci_tags` | `reference`, `max_results?` | List tags on **any** OCI registry (Docker Hub, GHCR, Quay, …). |
| `oci_manifest` | `reference` | Inspect a manifest: multi-arch platforms, or layers/size/config. |
| `artifacthub_search` | `query`, `kind?`, `max_results?` | Artifact Hub: Helm charts, Operators, krew, policies, Tekton. |

## Confirming destructive actions

Destructive tools (`docker_stop`/`docker_remove`/`docker_exec`/`docker_rmi`,
`k8s_delete`, `fs_delete`/`fs_move`, destructive `git_run` subcommands, and database
writes via `db_query`/`redis_command`) are **always exposed**, but the first call
performs nothing: it returns a one-time `confirm` token describing exactly what will
happen. Call the tool again with `confirm=<token>` to actually run it, or
`confirm=<token>, trust=true` to also stop being asked for that action for the rest
of the session. This works on **any** MCP client (no elicitation support required).
Setting the family's `allow_destructive` pre-authorizes the action and skips the
prompt entirely. (Tokens expire after 5 minutes and are single-use.)

## Local Docker daemon

A local-system capability (direct Engine API, no CLI), gated by `[docker]` — on by
default. Destructive tools confirm at call time (see above); `allow_destructive`
pre-authorizes them. Full detail:
[containers.md](containers.md#local-docker-daemon-write-access).

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `docker_ps` | `all?` | read | List containers. |
| `docker_images` | — | read | List local images. |
| `docker_inspect` | `container` | read | Full container JSON. |
| `docker_logs` | `container`, `tail?` | read | Container logs. |
| `docker_info` | — | read | Daemon version + state. |
| `docker_pull` | `image` | write | Pull an image. |
| `docker_run` | `image`, `name?`, `command?` | write | Create + start a container. |
| `docker_start` | `container` | write | Start a stopped container. |
| `docker_build` | `context`, `tag`, `dockerfile?` | write | Build an image from a context dir. |
| `docker_stop` | `container`, `confirm?`, `trust?` | destructive | Stop a container (confirm first). |
| `docker_remove` | `container`, `force?`, `confirm?`, `trust?` | destructive | Remove a container (confirm first). |
| `docker_exec` | `container`, `command`, `confirm?`, `trust?` | destructive | Run a command in a container (confirm first). |
| `docker_rmi` | `image`, `force?`, `confirm?`, `trust?` | destructive | Remove an image (confirm first). |

## Kubernetes cluster

Cluster control via the API (kube-rs, reads kubeconfig; no `kubectl`), gated by
`[kubernetes]` — on by default; `k8s_delete` confirms at call time (see above),
`allow_destructive` pre-authorizes it. Full detail:
[containers.md](containers.md#kubernetes-cluster).

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `k8s_contexts` | — | read | List kubeconfig contexts + current. |
| `k8s_get` | `kind`, `name?`, `namespace?` | read | Get one object or list a kind. |
| `k8s_describe` | `kind`, `name`, `namespace?` | read | Full JSON of one object. |
| `k8s_logs` | `pod`, `namespace?`, `container?`, `tail?` | read | A pod's logs. |
| `k8s_apply` | `manifest` | write | Apply a kubefile (multi-doc YAML). |
| `k8s_scale` | `kind`, `name`, `replicas`, `namespace?` | write | Scale a workload. |
| `k8s_delete` | `kind`, `name`, `namespace?`, `confirm?`, `trust?` | destructive | Delete a resource (confirm first). |

## Local filesystem

Read/edit files on the machine. **Off by default** — gated by `[filesystem]`
(`enabled`). Destructive ops (delete/move) confirm at call time (see above);
`allow_destructive` pre-authorizes them. Every path is confined to
`[filesystem].roots` (default: the working directory); `..` and symlink escapes are
rejected. See [`config/10-filesystem.toml`](../config/10-filesystem.toml).

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `fs_read` | `path`, `max_chars?` | read | Read a file's text. |
| `fs_list` | `path?` | read | List a directory's entries. |
| `fs_stat` | `path` | read | A path's type/size/modified/read-only. |
| `fs_find` | `pattern`, `path?` | read | Find files by name (`*` glob or substring). |
| `fs_write` | `path`, `content` | write | Create/overwrite a file. |
| `fs_edit` | `path`, `old_string`, `new_string`, `replace_all?` | write | Replace text in a file. |
| `fs_mkdir` | `path` | write | Create a directory (with parents). |
| `fs_delete` | `path`, `recursive?`, `confirm?`, `trust?` | **destructive** | Delete a file/directory (confirm first). |
| `fs_move` | `source`, `dest`, `confirm?`, `trust?` | **destructive** | Move/rename a path (confirm first). |

## Shell (arbitrary code execution)

`shell_run` runs commands on the machine. **Off by default** — the most dangerous
tool. Gated by `[shell]`: in allowlist mode only programs in `[shell].allow` run
(executed directly, no shell, so metacharacters are inert); `[shell].allow_unrestricted`
runs anything via the system shell. Each run has a timeout + working dir. See
[`config/11-shell.toml`](../config/11-shell.toml).

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `shell_run` | `command`, `workdir?`, `timeout_secs?` | Run a command; returns exit code + stdout/stderr. |

## Git

`git_run` runs the local `git` binary (no shell) in a repository. On by default
(`[git]`); destructive subcommands (push/reset/clean/rebase/…) confirm at call time
(see above — `confirm`/`trust` args), and `[git].allow_destructive` pre-authorizes
them. See [`config/12-git.toml`](../config/12-git.toml).

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `git_run` | `args`, `repo?`, `confirm?`, `trust?` | Run `git <args>` (without the leading `git`); returns exit code + output. |

## Media conversion (FFmpeg)

Shells out to a local `ffmpeg`/`ffprobe`. **Off by default** (`[ffmpeg]`); needs
FFmpeg on `PATH`. Paths are confined to `[filesystem].roots`; `ffmpeg_convert`
confirms at call time (see above).

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `ffmpeg_probe` | `input` | read | Media metadata: format, duration, bitrate, per-stream codec/resolution/sample-rate. |
| `ffmpeg_convert` | `input`, `output`, `args?`, `confirm?`, `trust?` | write | Convert/transcode; extra ffmpeg flags via pre-split `args` (confirm first). |

## Spreadsheets (CSV / XLSX)

Read/query/write tabular files. **Off by default** (`[spreadsheet]`); paths confined
to `[filesystem].roots`; `sheet_write` confirms at call time (see above).

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `sheet_read` | `path`, `sheet?`, `max_rows?` | read | Read a CSV/TSV/XLSX/XLS/ODS sheet as a table. |
| `sheet_query` | `path`, `column`, `equals`, `sheet?`, `select?`, `max_rows?` | read | Filter rows by a header column == value, project columns. |
| `sheet_write` | `path`, `rows`, `sheet_name?`, `confirm?`, `trust?` | write | Write rows to CSV/TSV or XLSX (format by extension; confirm first). |

## System information

Read-only host facts, gated by `[sysinfo]` (on by default; cross-platform — Linux
`/proc`/`/sys`, Windows OS APIs). GPU stats use NVML and return a clear message when
no NVIDIA GPU / NVML library is present. See
[`config/13-sysinfo.toml`](../config/13-sysinfo.toml).

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `system_info` | — | Host name, OS/kernel, uptime, CPU (model/cores/usage), memory/swap. |
| `system_disks` | — | Mounted disks: filesystem, total/used/free space. |
| `system_gpu` | — | NVIDIA GPU name, memory, utilization, temperature (via NVML). |

## Devices (off by default)

Direct hardware access — both **off by default**; writes go through the confirmation
guard. See [`config/16-devices.toml`](../config/16-devices.toml).

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `serial_ports` | — | read | List serial ports (name + USB/Bluetooth/PCI type). |
| `serial_send` | `port`, `data`, `baud?`, `confirm?`, `trust?` | **side-effecting** | Write bytes to a serial port (confirm first). |
| `serial_read` | `port`, `baud?`, `timeout_ms?`, `max_bytes?` | read | Read from a serial port for a timeout (text + hex). |
| `printer_list` | — | read | List printers (CUPS / Windows spooler). |
| `printer_print` | `text`, `printer?`, `confirm?`, `trust?` | **side-effecting** | Print text (confirm first). |
| `sdr_devices` | — | read | List attached SDRs (RTL-SDR via `rtl_test`, HackRF via `hackrf_info`). |
| `sdr_scan` | `start_mhz`, `end_mhz`, `bin_khz?`, `top?` | read | Sweep the spectrum (`rtl_power`) and report the strongest bins. Receive-only. |

## Databases

Query configured PostgreSQL / MySQL / Redis instances. **Off by default** — the
tools appear only when at least one `[databases.<id>]` is configured (a URL is a
credential-bearing opt-in). Reads run immediately; writes/DDL (SQL) and write/admin
commands (Redis) confirm at call time (see above), and a per-instance
`allow_destructive` pre-authorizes them. See
[`config/14-databases.toml`](../config/14-databases.toml).

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `db_list` | — | read | List configured databases (id + kind; never URLs). |
| `db_query` | `database`, `sql`, `confirm?`, `trust?` | read / **destructive** | Run SQL on a postgres/mysql instance (writes confirm first). |
| `redis_command` | `database`, `command`, `confirm?`, `trust?` | read / **destructive** | Run a Redis command (writes confirm first). |

## Caching & file store

Most search/retrieval/lookup results are cached automatically (in-memory; optionally
Redis — see [`config/05-cache.toml`](../config/05-cache.toml)). An optional on-disk
**file store** (`[store]`, off by default — see
[`config/15-store.toml`](../config/15-store.toml)) caches fetched *bytes* (repo files,
PDFs, rendered pages) for reuse, with TTL + size retention.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `cache_status` | — | Report the in-memory search/retrieval caches and the file store (counts + size). Always available. |
| `store_fetch` | `url` | Download a URL and cache its bytes in the store (gated by `[store]`). |
| `store_get` | `key`, `max_chars?` | Read a stored entry's content as text. |
| `store_list` | — | List store entries (key, size, age). |
| `store_purge` | `key?` | Remove one entry, or purge the whole store. |

## Date & time

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `datetime` | `timezone?` | Current date/time (local + UTC + Unix), plus an optional IANA timezone. The model has no "now". |
| `date_diff` | `from`, `to?` | Difference between two dates (days/years, 'ago / from now'); `to` defaults to now. |
| `time_convert` | `time`, `to_tz`, `from_tz?` | Convert a date/time to another IANA timezone. |

## Language

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `translate` | `text`, `to`, `from?` | Translate text via Google Translate (keyless); `to` is an ISO-639 code, `from` auto-detects. |
| `detect_language` | `text` | Detect a text's language (ISO-639 code) via Google Translate (keyless). |

## Data, JSON & regex (local, no network)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `json_query` | `json`, `pointer?` | Parse/validate JSON; extract a value by RFC-6901 JSON Pointer, or pretty-print. |
| `json_format` | `json`, `minify?` | Pretty-print (default) or minify JSON. |
| `yaml_to_json` | `data` | Convert YAML → JSON. |
| `json_to_yaml` | `data` | Convert JSON → YAML. |
| `regex_search` | `pattern`, `text`, `all?`, `ignore_case?` | Find regex matches and their capture groups (Rust `regex` syntax). |
| `regex_replace` | `pattern`, `text`, `replacement`, `all?`, `ignore_case?` | Substitute regex matches (`$1`/`${name}` refs). |

## Math, science & units (local, no network)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `math_eval` | `expression` | Evaluate an arithmetic/scientific expression (sqrt, sin, pi, `^`, …). |
| `math_solve` | `equation` | Solve a linear/quadratic equation in `x` (e.g. `x^2 - 5x + 6 = 0`). |
| `geo_distance` | `lat1`, `lon1`, `lat2`, `lon2` | Great-circle distance between two coordinates (km + mi). |
| `geo_azimuth` | `lat1`, `lon1`, `lat2`, `lon2` | Initial bearing/azimuth (+ back azimuth, compass) between two coordinates. |
| `wave_frequency` | `frequency_hz?`, `wavelength_m?`, `speed_m_s?` | Convert frequency ↔ wavelength ↔ period (v = f·λ). |
| `forecast` | `values`, `horizon`, `season_length?` | Forecast a numeric series (Holt / Holt-Winters exponential smoothing) with an approximate interval. |
| `convert_units` | `value`, `from`, `to` | Convert between units (length/mass/volume/area/speed/time/data/temperature). |

## Finance & markets

Money math (local) plus keyless market data. Quotes are delayed/reference data, not
a live trading feed.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `currency_convert` | `amount`, `from`, `to` | Convert currencies via keyless ECB reference rates (Frankfurter); cached. |
| `compound_interest` | `principal`, `annual_rate_percent`, `years`, `compounds_per_year?` | Compound-interest future value + interest earned. |
| `loan_payment` | `principal`, `annual_rate_percent`, `months` | Amortized monthly payment, total paid, total interest. |
| `stock_quote` | `symbol` | Delayed stock/index/FX quote via keyless Stooq (OHLC + volume). |
| `yahoo_quote` | `symbol` | Delayed Yahoo Finance quote: price, change/%, day & 52-week range, volume, currency, exchange. |
| `yahoo_history` | `symbol`, `range?`, `interval?` | OHLC price history (date, O/H/L/C, volume) from Yahoo; pick range (1d…max) + bar interval. |
| `yahoo_search` | `query` | Resolve a company name / partial ticker to Yahoo symbols (type + exchange). |

## Space & astronomy (keyless)

NASA works keyless via `DEMO_KEY`; set `[nasa].key` (`LODESTONE_NASA_KEY`) to raise
the rate limit.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `nasa_apod` | `date?` | NASA Astronomy Picture of the Day (title, image/video URL, explanation). |
| `nasa_neo` | `date?` | Near-Earth objects for a day (diameter, hazardous flag, miss distance, velocity). |
| `nasa_mars_photos` | `rover?`, `sol?`, `earth_date?`, `max_results?` | Mars rover photo URLs (camera + date). |
| `sat_tle` | `query` | Fetch a satellite's current TLE from CelesTrak (by NORAD id or name). |
| `sat_position` | `tle_line1`, `tle_line2`, `at?` | SGP4 sub-point: latitude, longitude, altitude, speed. |
| `sat_observe` | `tle_line1`, `tle_line2`, `observer_lat`, `observer_lon`, `observer_alt_km?`, `at?` | Azimuth/elevation/range from an observer. |

## Background tasks (off by default)

Run long work off the request path and poll for results (model-polled — works on any
client). Gated by `[tasks]`. Currently backgrounds searches.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `task_run` | `op?`, `kind`, `query`, `max_results?` | Start a background search (`kind` = web/code/docs/qa); returns a task id. |
| `task_list` | — | List background tasks (id, status, label, age). |
| `task_status` | `id` | A task's status (running/done/failed/cancelled). |
| `task_result` | `id` | A task's result (or still-running / error). |
| `task_cancel` | `id` | Cancel a running task. |

## Meta

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `list_providers` | — | Show the active providers, strategy, and ranking. |
| `constellation_status` | — | Show the peer-to-peer constellation graph (peers, machine ids, reputation, edges); says disabled when off. |
| `constellation_peers` | — | List constellation nodes and how many **hops** away each is (direct = 1), with machine id/reputation. |
| `constellation_seeds` | — | Per-blob **seed ratio** (bytes served to peers vs. fetched from them), BitTorrent-style. |

## Per-provider

One direct tool per *configured* provider, named `<kind>_<id>` (e.g. `web_mojeek`,
`code_github`, `qa_stackoverflow`, `docs_react`, `docs_kubernetes`). Args: `query`,
`max_results?`, `language?`, `site?`, `render?`. Targets a single source, bypassing
the chain/strategy. Generated from your config and gateable via `[tools]`.

StackOverflow adds one bespoke provider skill: `qa_stackoverflow_answers`
(`question`, `site?`, `max_answers?`, `render?`) — read a question's body + top
answers (with code). Uses the keyless API by default; `render=true` scrapes the
stackoverflow.com page via the headless browser instead (saves API quota;
`stackoverflow` site only).

---

**Typical flow:** *search* (`web_search` / `code_search` / `docs_search` /
`qa_search`) → *retrieve* the best hit (`fetch_page` / `render_page` /
`fetch_repo_file` / `qa_stackoverflow_answers`).
