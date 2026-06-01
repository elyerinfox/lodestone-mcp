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
| `ncbi_search` | `db`, `query`, `max_results?` | Search ANY NCBI database (pmc, gene, protein, nucleotide, snp, clinvar, taxonomy, books, mesh, …) via E-utilities; UIDs + key fields + ncbi.nlm.nih.gov link. |
| `ncbi_summary` | `db`, `id` | One NCBI record's summary fields + link. |
| `unpaywall_lookup` | `doi` | Best LEGAL open-access PDF + all OA locations for a DOI (Unpaywall; needs `LODESTONE_CONTACT_EMAIL`). |
| `openalex_search` | `query`, `max_results?` | Search OpenAlex works: authors/year/venue/DOI + OA PDF link. |
| `openalex_work` | `id` | One OpenAlex work by DOI or id, with OA status + PDF. |
| `hf_model_search` | `query`, `max_results?` | Search the Hugging Face Hub for models (by downloads). |
| `hf_dataset_search` | `query`, `max_results?` | Search the Hugging Face Hub for datasets (by downloads). |
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
`k8s_delete`, `fs_delete`/`fs_move`, destructive `git_run` subcommands, every
`shell_run` (arbitrary code), `ffmpeg_convert`, `sheet_write`, `serial_send`,
`printer_print`, and database writes via `db_query`/`redis_command`) are **always
exposed** (when their family is enabled), but the first call
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
runs anything via the system shell. Because it's arbitrary code, **every call confirms
at call time** (see above): the first call returns a token and runs nothing; call again
with `confirm=<token>` (or `confirm` + `trust=true` to whitelist that exact command).
`[shell].allow_destructive` pre-authorizes. Each run has a timeout + working dir. See
[`config/11-shell.toml`](../config/11-shell.toml).

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `shell_run` | `command`, `workdir?`, `timeout_secs?`, `confirm?`, `trust?` | Run a command (confirm first); returns exit code + stdout/stderr. |

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
| `system_gpu_nvidia` | — | NVIDIA GPU name, memory, utilization, temperature (via NVML). |
| `system_gpu_amd` | — | AMD GPU model, VRAM, busy %, temperature (Linux DRM sysfs via `amdgpu`). |
| `system_gpu_intel` | — | Intel GPU model, frequency, temperature (Linux DRM sysfs via `i915`/`xe`). |
| `system_os_release` | — | Parse `/etc/os-release` (Linux distro identifier per the systemd spec). |

## Messaging (off by default)

MQTT pub/sub against a configured broker, plus the Meshtastic LoRa mesh decoder
that rides on it. Both **off by default**. See
[`config/19-mqtt.toml`](../config/19-mqtt.toml) and
[`config/20-meshtastic.toml`](../config/20-meshtastic.toml).

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `mqtt_publish` | `topic`, `payload?` or `payload_base64?`, `qos?`, `retain?` | Publish a message to the broker. |
| `mqtt_subscribe` | `topic`, `qos?` | Subscribe (supports `+` / `#` wildcards); buffer fills as messages arrive. |
| `mqtt_unsubscribe` | `topic` | Drop a prior subscription. |
| `mqtt_recent` | `topic?`, `limit?` | Recent buffered messages, newest first. |
| `mqtt_status` | — | Broker URL, credentials presence (`<set>`/`<unset>`), subscriptions, buffer size. |
| `meshtastic_messages` | `channel?`, `from?`, `limit?` | Text messages decoded from the Meshtastic JSON-over-MQTT topic format. |
| `meshtastic_nodes` | — | Mesh nodes recently heard (id / longname / shortname / RSSI / SNR / last-seen). |
| `meshtastic_send` | `text`, `channel?`, `region?`, `to?` | Publish a text message onto the mesh through the bridging node. |
| `meshtastic_status` | — | Transport, topic root, defaults, MQTT-wiring status, mesh buffer count. |

## Package managers (off by default)

OS / distro package managers — one set of tools that target each PM via an
explicit `kind` argument. Off by default `[packages]`. Destructive ops
(`install`/`upgrade`/`remove`) go through the confirmation guard. **No `sudo`** —
privilege is the operator's choice. See
[`config/21-packages.toml`](../config/21-packages.toml) and
[`docs/skills/packages.md`](skills/packages.md).

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `package_managers` | — | read | List supported PMs with ✓ / · for whether the binary is on `$PATH`. |
| `package_search` | `kind`, `query` | read | PM-native search. |
| `package_info` | `kind`, `name` | read | PM-native package metadata. |
| `package_list` | `kind` | read | Installed packages. |
| `package_updates` | `kind` | read | Available updates (without applying). |
| `package_install` | `kind`, `name`, `confirm?`, `trust?` | **destructive** | Install via the named PM. |
| `package_upgrade` | `kind`, `name?`, `confirm?`, `trust?` | **destructive** | Upgrade one package (or all when `name` is omitted). |
| `package_remove` | `kind`, `name`, `confirm?`, `trust?` | **destructive** | Remove a named package. |

`kind` values: `winget`, `chocolatey` (alias `choco`), `brew` (alias `homebrew`),
`apt` (alias `apt-get`), `dnf`, `yum`, `apk`, `pacman`, `yay` (alias `aur`),
`zypper`, `pkg`.

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

Query PostgreSQL / MySQL / Redis. **Off by default** (`[databases].enabled`); **no
preconfiguration** — you pass a `connection` URL in each call (the engine is inferred
from its scheme), so connections come from the conversation, never stored config.
URLs are secrets (never logged). Reads run immediately; writes/DDL (SQL) and
write/admin commands (Redis) confirm at call time (see above); `[databases].
allow_destructive` pre-authorizes. See
[`config/14-databases.toml`](../config/14-databases.toml).

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `db_query` | `connection`, `sql`, `confirm?`, `trust?` | read / **destructive** | Run SQL on a `postgres://`/`mysql://` connection (writes confirm first). |
| `redis_command` | `connection`, `command`, `confirm?`, `trust?` | read / **destructive** | Run a Redis command on a `redis://` connection (writes confirm first). |

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

## Math & science (local, by field)

Each field has a named-formula registry: call `<field>_formula` with `name` + an
`args` `{var: value}` map (SI units, angles in degrees), and `<field>_formula_list`
to discover ids/equations/signatures.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `arithmetic_eval` | `expression` | Evaluate a free-form expression (sqrt, sin, pi, `^`, …). |
| `algebra_solve` | `equation` | Solve a linear/quadratic equation in `x` (e.g. `x^2 - 5x + 6 = 0`). |
| `algebra_formula` / `algebra_formula_list` | `name`, `args` / `filter?` | Combinatorics + algebra (nPr, nCr, factorial, discriminant). |
| `geometry_formula` / `geometry_formula_list` | `name`, `args` / `filter?` | Areas, volumes, Pythagoras, Heron, law of cosines, distances. |
| `geo_distance` | `lat1`, `lon1`, `lat2`, `lon2` | Great-circle distance between two coordinates (km + mi). |
| `geo_azimuth` | `lat1`, `lon1`, `lat2`, `lon2` | Initial bearing/azimuth (+ back azimuth, compass) between two coordinates. |
| `trig_formula` / `trig_formula_list` | `name`, `args` / `filter?` | sin/cos/tan + inverses (degrees), deg↔rad, law of sines/cosines, arc/sector. |
| `physics_formula` / `physics_formula_list` | `name`, `args` / `filter?` | ~70 physics formulas (mechanics, gravitation, EM, thermo, waves/optics, relativity, atomic/nuclear, fluids). |
| `physical_constant` | `name?` | SI physical constants (c, G, h, k_B, R, …). |
| `wave_frequency` | `frequency_hz?`, `wavelength_m?`, `speed_m_s?` | Convert frequency ↔ wavelength ↔ period (v = f·λ). |
| `forecast_holt_linear` | `values`, `horizon`, `alpha?`, `beta?` | Forecast a numeric series with Holt's linear trend (level + trend), approximate interval. |
| `forecast_holt_winters` | `values`, `horizon`, `season_length`, `alpha?`, `beta?`, `gamma?` | Forecast a seasonal series with Holt-Winters additive (level + trend + season). |
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
| `nasa_neo` | `date?` | Near-Earth objects for a day (diameter, hazardous flag, miss distance, velocity). |
| `nasa_mars_photos` | `rover?`, `sol?`, `earth_date?`, `max_results?` | Mars rover photo URLs (camera + date). |
| `sat_tle` | `query` | Fetch a satellite's current TLE from CelesTrak (by NORAD id or name). |
| `sat_position` | `tle_line1`, `tle_line2`, `at?` | SGP4 sub-point: latitude, longitude, altitude, speed. |
| `sat_observe` | `tle_line1`, `tle_line2`, `observer_lat`, `observer_lon`, `observer_alt_km?`, `at?` | Azimuth/elevation/range from an observer. |

## Async search (off by default)

Launch a search in the background and get a `task_id`. Manage via the MCP-spec
`tasks_*` tools below (`tasks_list` / `tasks_get` / `tasks_result` / `tasks_cancel`)
— they read the same runtime. Gated by `[tasks]`.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `search_async` | `kind` (`web`/`code`/`docs`/`qa`), `query`, `max_results?` | Start a background search; returns a `task_id`. |

## MCP Tasks primitive (always on)

The 2025-11-25 spec's task management methods (`tasks/list`, `tasks/get`,
`tasks/result`, `tasks/cancel`) exposed as tools so every MCP client can
drive them today. Backed by the same `TaskRuntime` as `search_async`,
`mqtt_listen`, and `meshtastic_listen`.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `tasks_list` | — | List tracked async tasks (newest first). |
| `tasks_get` | `task_id` | One task's metadata (status, last progress, timestamps). |
| `tasks_result` | `task_id` | Terminal result (or in-progress log replay). |
| `tasks_cancel` | `task_id` | Cancel a running task; pushes `notifications/tasks/status`. |

## Persistent memory & solutions (on by default `[memory]`)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `remember` | `text`, `as?`, `scope?`, `tags?` | Frictionless write — auto-derives a key (kebab-case from first words + short hash), auto-extracts tags. Defaults to a memo; text shaped like a recipe (`→`, starts with `to`/`when`/`if`/`fix:`/`solution:`/`use`) auto-classifies as a solution. Override with `as: "fact" \| "solution"`. |
| `remember_fact` | `text`, `scope?`, `tags?` | Always writes a memo. No classifier. |
| `remember_solution` | `text`, `problem?`, `summary?`, `tags?` | Always writes a solution. First sentence becomes the problem, rest becomes the content, unless explicitly overridden. |
| `recall` | `query`, `kinds?`, `limit?` | One merged hit list across memos + solutions + phrasings. Each row tagged by kind. Replaces calling `memory_search` and `solution_find` separately. |
| `memory_save` | `key`, `value`, `scope?`, `tags?` | Save/upsert a key→value memory persisted across sessions. |
| `memory_get` | `key`, `scope?` | Exact lookup by key (and optional scope). |
| `memory_list` | `scope?`, `prefix?`, `max?` | List memories with previews, newest-updated first. |
| `memory_search` | `query`, `scope?`, `tag?`, `max?` | Substring search across key/value/tags. |
| `memory_forget` | `key`, `scope?`, `confirm?`, `trust?` | Delete a memory. **Destructive — guarded.** |
| `solution_record` | `problem`, `summary`, `content`, `notes?`, `tags?` | Record a proposed solution; returns its id. |
| `solution_find` | `query?`, `tags?`, `max?` | Surface SUGGESTED prior solutions (advisory). Exact canonical > exact concept > fuzzy Jaccard > substring, plus tag boost. |
| `solution_show` | `id` | Show one solution with its full revision history. |
| `solution_list` | `max?` | List recorded solutions, newest-updated first. |
| `solution_update` | `id`, `summary`, `content`, `notes?`, `tags?` | Append a new revision; prior revisions are kept. Pass `tags` to replace, `[]` to clear. |
| `solution_forget` | `id`, `confirm?`, `trust?` | Delete a solution (drops all revisions). **Destructive — guarded.** |
| `solution_link` | `from`, `kind`, `to`, `note?` | Declare a typed relation between two solutions; reciprocal is auto-added (e.g. `supersedes`→`superseded-by`). |
| `solution_unlink` | `from`, `kind`, `to` | Remove a typed link; the reciprocal on the target is also removed. |
| `solution_graph` | `id`, `depth?` | BFS subgraph around one solution (default 2 hops, max 5), showing typed edges to every reachable solution. |
| `solution_related` | `id`, `max?` | Rank solutions related to one source, combining explicit links + shared tags + concept-token Jaccard overlap. |
| `solution_alias_add` | `id`, `phrasing` | Attach an alternate phrasing of the same underlying question — recall (token + semantic) then considers it, so future differently-worded queries still find the solution. |
| `solution_alias_remove` | `id`, `phrasing` | Detach a previously-added phrasing (match is by canonical form). |
| `conversation_list` | `max?` | List recorded conversations, most recently active first; turn count, started/last-seen, first query preview. |
| `conversation_show` | `id`, `max?` | Walk one conversation: every tool call (chronological), with query + a short response excerpt + the list of solutions whose revisions it produced. |
| `solution_conversations` | `id` | List the conversation(s) a solution came from, grouped by which revisions each one produced (many-to-many via revisions). |
| `conversation_forget` | `id`, `confirm?`, `trust?` | Delete one conversation (cascades to turns; NULLs revisions' back-pointer). **Destructive — guarded.** |
| `conversation_prune` | `older_than_days?`, `keep_newest?`, `dry_run?`, `confirm?`, `trust?` | Bulk-delete by retention policy. Falls back to configured `[memory].conversation_retention_days` / `max_conversations` when no args given. **Destructive — guarded; `dry_run=true` bypasses.** |

## Charts & plots (on by default `[chart]`)

Pure-Rust SVG charts (no headless browser, no network) plus an interactive HTML
escape hatch and a mermaid passthrough. SVGs ship as MCP `image/svg+xml` so
clients render inline, with a one-line text fallback for clients that can't.
All static charts are responsive via `viewBox`. Full detail:
[skills/chart.md](skills/chart.md).

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `chart_line` | `series`, `title?`, `xlabel?`, `ylabel?`, `width?`, `height?` | Multi-series line plot (tab10 palette + legend). `x` accepts numbers **or** ISO-8601 date strings. |
| `chart_bar` | `labels`, `values`, `title?`, … | Vertical bar chart. |
| `chart_scatter` | `points`, `title?`, `point_size?`, … | Scatter plot. `x` accepts numbers or ISO-8601 dates. |
| `chart_histogram` | `values`, `bins?`, `title?`, … | Histogram, auto-bins to √n when `bins` omitted. |
| `chart_pie` | `slices`, `title?`, … | Pie chart with percentage legend. |
| `chart_heatmap` | `matrix`, `row_labels?`, `col_labels?`, `colormap?`, … | 2D matrix as colored cells with a colorbar. Colormaps: viridis (default), magma, plasma, coolwarm, grayscale. |
| `chart_grafana` | `title?`, `series`, `unit?`, … | Dark-themed time-series panel — translucent area fills + last-value labels. Operational-telemetry feel. |
| `chart_stat` | `value`, `label?`, `unit?`, `thresholds?`, `sparkline?`, `color_mode?` | Grafana Stat panel: big-number tile, threshold-tinted, optional background sparkline. |
| `chart_gauge` | `value`, `min`, `max`, `thresholds?`, `unit?`, `title?` | Grafana Gauge: 270° radial dial with threshold bands. |
| `chart_bar_gauge` | `items`, `min`, `max`, `thresholds?`, `unit?` | Grafana Bar gauge: one horizontal threshold-tinted bar per item. |
| `chart_state_timeline` | `rows`, `state_colors?` | Grafana State timeline: categorical state bands per row (UP / DEGRADED / DOWN). |
| `chart_candlestick` | `candles`, `up_color?`, `down_color?`, … | Grafana Candlestick: OHLC bodies + wicks for financial time-series. |
| `chart_sparkline` | `points`, `color?`, `fill_opacity?`, … | Tiny inline trend, no chrome. |
| `chart_canvas` | `commands`, `width?`, `height?`, `background?`, `title?` | Procedural canvas (turtle / Logo style): `line` / `rect` / `circle` / `polygon` / `polyline` / `text` drawn in order. |
| `chart_interactive` | `library` (chartjs/plotly), `config`, `title?`, `width?`, `height?` | Self-contained HTML wrapping Chart.js or Plotly. Clients that render HTML get full interactivity (hover, zoom, pan); others see source. |
| `chart_mermaid` | `source`, `title?` | Wrap mermaid source in a markdown code fence. Modern MCP clients render mermaid natively. |

## HTML render & diagnostics (on by default `[html]`)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `html_render` | `html?`, `url?`, `wait_ms?` | Execute an HTML snippet OR a URL in the shared headless Chrome, wait for JS, and return diagnostics: every `console.*` call (level + args + source/line), every uncaught JS exception (text + stack), every network failure (DNS / refused / CORS), every HTTP 4xx/5xx response, final title / URL / elapsed time. Pair with `chart_interactive` output before shipping. |

## Image forensics + EXIF (on by default `[image]`)

Read-only, paths confined to `[filesystem].roots`. The "analyze" tools walk every
container marker / chunk so editor-vs-camera divergence stands out.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `image_info` | `path` | Format / dimensions / color / animation from structural headers (JPEG SOFn, PNG IHDR, GIF LSD, WebP VP8 / VP8L / VP8X, BMP DIB, TIFF, HEIF, JPEG-XL). Pure binary parse, no full decode. |
| `image_exif` | `path` | Full EXIF dump (IFD0 / Exif / GPS / Interop) via `kamadak-exif`. GPS decoded to signed decimal degrees + OSM link. Forensic divergence flags fire when `DateTime{Original,Digitized}` disagree or `Software` is editor-branded (Photoshop / GIMP / Lightroom / …). |
| `image_jpeg_analyze` | `path` | Walk every JPEG marker — APP segments by identifier (JFIF / EXIF / XMP / ICC_PROFILE / 8BIM / MPF / Adobe), DQT (encoder fingerprint), DHT, DRI, SOFn payload, SOS. |
| `image_png_analyze` | `path` | Walk every PNG chunk with decoded payloads — IHDR, tEXt / iTXt / zTXt, eXIf, iCCP, tIME, pHYs (with DPI conversion), gAMA, sRGB, acTL (APNG). Flags unknown private chunks. |

## FCC / amateur radio reference (on by default `[fcc]`)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `fcc_callsign` | `callsign` | US amateur callsign lookup via the keyless callook.info JSON API. Returns name, class, trustee, FRN, grant / expire dates, grid square. Non-amateur callsigns get a ULS web-search hint. |
| `fcc_amateur_bands` | `band?`, `license_class?` | Full US amateur band plan (2200m → 1.25cm, 24 bands) with per-class privileges. `band` matches wavelength (`40m`), region (`HF`), or a frequency in MHz (`14.250`). |
| `fcc_radio_service` | `service?`, `channel?` | FRS / GMRS / MURS / CB reference: license, power, channels, antenna rules, spectrum sharing. `service="compare"` for the side-by-side table. |

## Weather, geo & infrastructure (keyless)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `weather_forecast` | `lat`, `lon`, `hourly?`, `daily?`, `model?`, `forecast_days?`, `hours?`, `timezone?` | Open-Meteo point forecast. Selectable NWP model: `best_match` (default), `gfs_seamless`, `ecmwf_ifs04` / `ecmwf_ifs025`, `icon_seamless`, `gem_seamless`, `jma_seamless`, `metno_seamless`, `ukmo_seamless`, `arpege_seamless`. |
| `weather_marine` | `lat`, `lon`, `hourly?`, `days?`, `timezone?` | Marine forecast: wave height, period, swell, sea-surface temperature. |
| `weather_air_quality` | `lat`, `lon`, `hourly?`, `days?`, `timezone?` | Air quality forecast: PM10 / PM2.5 / NO₂ / O₃ / CO / SO₂, European AQI, dust, pollen. |
| `weather_historical` | `lat`, `lon`, `start_date`, `end_date`, `hourly?`, `daily?`, `timezone?` | ERA5 reanalysis archive (1940 → ~5 days ago). |
| `noaa_alerts` | `area?`, `status?`, `max?` | Active NWS weather alerts (US). |
| `noaa_forecast` | `lat`, `lon`, `hourly?` | NWS point forecast (US). |
| `osm_geocode` | `query`, `max_results?` | Nominatim place-name → lat/lon. |
| `osm_reverse_geocode` | `lat`, `lon` | Nominatim lat/lon → address. |
| `osm_overpass` | `query`, `max_elements?` | Run an Overpass-QL query against OpenStreetMap. |
| `osm_elevation` | `points` | Open-Elevation ground-elevation lookup (≤ 100 points). |
| `osm_route` | `from_lat`, `from_lon`, `to_lat`, `to_lon`, `profile?` | OSRM public demo routing. |
| `grid_power_plants` | `south`, `west`, `north`, `east`, `max?` | Power plants in a bounding box (OSM). |
| `grid_transmission_lines` / `grid_substations` / `grid_data_centres` / `grid_gas_pipelines` / `grid_submarine_cables` | bbox + `max?` | Critical-infrastructure layers (OSM Overpass). |
| `peeringdb_network` | `asn?`, `name?`, `max?` | PeeringDB network (ASN) lookup. |
| `peeringdb_ix` | `name?`, `country?`, `city?`, `max?` | Internet exchanges. |
| `peeringdb_facility` | `name?`, `country?`, `city?`, `max?` | Colo facilities. |
| `peeringdb_org` | `name?`, `max?` | Organizations. |

## Binary / signal / pcap / notebook (off by default — read-only)

Paths confined to `[filesystem].roots`. Pair `signal_*` with `wave_*` for
FFT-of-decoded-audio.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `binary_info` | `path` | Identify ELF / PE / Mach-O / WASM / archive; arch, entry, sections (via `object`). |
| `binary_strings` | `path`, `min_len?`, `max?` | Extract printable strings. |
| `binary_entropy` | `path`, `block_size?` | Shannon entropy per block — spots packed / encrypted regions. |
| `binary_hexdump` | `path`, `offset?`, `length?` | Hexdump a range of bytes. |
| `signal_fft` | `samples`, `sample_rate_hz`, `window?` | Real-to-complex FFT (rustfft, runtime SIMD). Returns magnitude spectrum. |
| `signal_dominant_frequencies` | `samples`, `sample_rate_hz`, `top?`, `window?` | Top-K peak frequencies. |
| `signal_rms` | `samples` | Root-mean-square (signal level). |
| `signal_window` | `n`, `kind` | Compute a window: Hann / Hamming / Blackman / rectangular. |
| `wave_info` | `path` | `.wav` header summary (sample rate, bit depth, channels, duration). |
| `wave_samples` | `path`, `max_samples?`, `channel?` | Decode raw samples (for FFT / RMS). |
| `pcap_info` | `path` | `.pcap` header + packet count. |
| `pcap_packets` | `path`, `offset?`, `max?` | Walk packet records. |
| `disasm_x86_hex` | `hex`, `bits?`, `base?`, `max?` | Disassemble inline hex bytes (16/32/64-bit, `iced-x86`, NASM flavor). |
| `disasm_x86_file` | `path`, `offset?`, `length?`, `bits?` | Disassemble a region of a local binary. |
| `notebook_info` | `path` | `.ipynb` summary (cells, kernel, language). |
| `notebook_cells` | `path`, `max?`, `include_outputs?` | Walk cells (markdown + code; optionally outputs). |

## Python runner (off by default `[python]`)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `python_run` | `code`, `stdin?`, `args?`, `timeout_secs?`, `confirm?`, `trust?` | Execute Python via the configured interpreter (`[python].interpreter`). **Destructive — guarded.** |

## systemd (Linux, off by default `[systemd]`)

| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `systemd_list` | `state?`, `pattern?` | read | List units (active / failed / loaded / pattern-match). |
| `systemd_status` | `unit` | read | Unit status (`systemctl status`). |
| `systemd_show` | `unit`, `properties?` | read | Show one or more unit properties. |
| `systemd_start` / `systemd_stop` / `systemd_restart` | `unit`, `confirm?`, `trust?` | **destructive** | Start / stop / restart a unit. Guarded. |

## Energy (`[eia]`, keyless API but requires a free key)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `eia_browse` | `path?` | Browse the EIA v2 dataset tree (electricity / NG / petroleum / coal / renewables / international). |
| `eia_series` | `path`, `frequency?`, `data?`, `facets?`, `start?`, `end?` | Pull a specific time series. |

## Astronomy & radio (off by default)

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `astro_sun` | `lat`, `lon`, `date?` | Sun position (alt / az) + sunrise / transit / sunset for a date / location. Local compute. |
| `astro_moon` | `lat`, `lon`, `date?` | Moon position + rise / set + phase. |
| `radio_fspl` | `frequency_hz`, `distance_m` | Free-space path loss in dB. |
| `radio_link_budget` | `tx_power_dbm`, `tx_gain_dbi`, `rx_gain_dbi`, `frequency_hz`, `distance_m`, `cable_loss_db?`, `other_losses_db?` | Compute received power and link margin against a noise floor. |
| `radio_antenna` | `frequency_hz`, `gain_dbi?`, `effective_aperture_m2?` | Convert antenna gain ↔ effective aperture. |

## Browser sessions (on by default when Chrome/Chromium is available)

See [`docs/skills/browser_session.md`](skills/browser_session.md) for the
session / persona / guest-session conceptual split.

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `browser_open` | — | Open a new isolated Chromium tab; returns a `session_id`. |
| `browser_navigate` | `session_id`, `url`, `observe?` | Navigate the session; waits 15 s for nav to settle. `observe` is `"none"` / `"tree"` / `"screenshot"` / `"both"`. |
| `browser_click` | `session_id`, `selector`, `observe?` | Click the first element matching `selector`. |
| `browser_type` | `session_id`, `selector`, `text`, `submit?`, `observe?` | Focus + type. `submit: true` calls `form.requestSubmit()`. |
| `browser_wait` | `session_id`, `selector`, `timeout_ms?` | Poll until the selector exists, or time out. Returns `{matched}`. |
| `browser_extract` | `session_id`, `selector`, `attr?`, `limit?` | innerText or attribute for every match; capped at `limit`. |
| `browser_eval` | `session_id`, `script` | Arbitrary JS, returns JSON. Refused on guest sessions. |
| `browser_screenshot` | `session_id`, `full_page?` | Viewport (or full scroll) PNG as base64. |
| `browser_list` | — | Every open session with URL + title + age + idle. |
| `browser_close` | `session_id` | Dispose the tab and its isolated context. |
| `browser_persona_get` | `name` | Get-or-create the named LOCAL persona's session; cookies persist across calls. |
| `browser_persona_list` | — | List LOCAL personas only (guest sessions are dashboard-only). |
| `browser_persona_reset` | `name` | Force a fresh session on the named persona — state returns to `healthy`. |
| `browser_persona_delegate` | `persona_name`, `url` | Ask a constellation peer (with `[network.capabilities].browser = true`) to run a navigate on ITS persona. Sessions don't transport; the peer's SSRF guard refuses local-network URLs. |

## Meta

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `features` | `name?` | Per-family enabled/disabled status plus every knob (allow_destructive, recall thresholds, retention policy, embedding endpoint, …) and live memory counts. With `name=<family>`, focused dump for one family. Use BEFORE assuming a family is reachable. |
| `list_providers` | — | Show the active providers, strategy, and ranking. |
| `constellation_status` | — | Show the peer-to-peer constellation graph (peers, machine ids, reputation, edges); says disabled when off. |
| `constellation_peers` | — | List constellation nodes and how many **hops** away each is (direct = 1), with machine id/reputation. |
| `constellation_seeds` | — | Per-blob **seed ratio** (bytes served to peers vs. fetched from them), BitTorrent-style. |
| `constellation_capabilities` | `cap?` | Per-feature opt-in set every node advertises (`query` / `retrieval` / `blob` / `browser`). With `cap=<name>`, filter to nodes that have the named capability ON — answers "who can do browser work?". |

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
