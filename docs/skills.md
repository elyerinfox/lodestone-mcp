# Skills reference

A **skill** is one self-contained tool family the server exposes to the model. Every
skill is a module under [`src/skills/`](../src/skills/) implementing the shared
`Skill` contract (`name` / `description` / `schema` / `call`); `main.rs` holds no
tool logic. This page is the **index** — each skill has its own page under
[`docs/skills/`](skills/) with its tools, arguments, config/gating, and example
uses. For the flat table of *every* tool see [tools.md](tools.md); for *data sources*
behind the search tools see [providers.md](providers.md).

## How skills are gated

- **`[tools]`** — any tool can be allow/deny-listed (`config/01-tools.toml`).
- **Family switches** — local-system families have their own `enabled` flag
  (`[docker]`, `[kubernetes]`, `[filesystem]`, `[shell]`, `[git]`, `[sysinfo]`,
  `[serial]`, `[printer]`, `[store]`, `[databases.<id>]`, `[network]`). Filesystem,
  shell, serial, printer, databases, and the file store are **off by default**.
- **Destructive confirmation** (golden rule 8) — destructive actions are *exposed*
  but never fire unguarded: the first call returns a one-time `confirm` token and
  does nothing; call again with `confirm=<token>` (or `confirm` + `trust=true` to
  whitelist it for the session). A family's `allow_destructive` pre-authorizes
  (skips the prompt). Client-agnostic — no MCP elicitation required. See
  [tools.md → Confirming destructive actions](tools.md#confirming-destructive-actions).
- **Keyless by default** — everything works with no accounts/keys; optional
  credentials only raise limits or unlock keyed sources.

## Search & retrieval

| Skill | Tools | What |
| --- | --- | --- |
| [search](skills/search.md) | `web_search`, `code_search`, `docs_search`, `qa_search`, `<kind>_<id>`, `qa_stackoverflow_answers` | Run the provider registry; per-provider tools; SO answers. |
| [retrieve](skills/retrieve.md) | `fetch_page`, `render_page`, `webpage_to_pdf`, `read_pdf`, `fetch_repo_file` | Read a page/PDF/repo file (HTTP or headless render). |
| [archive](skills/archive.md) | `wayback_fetch` | Read a page's Wayback Machine snapshot. |

## Knowledge & references (keyless)

| Skill | Tools | What |
| --- | --- | --- |
| [rfc](skills/rfc.md) | `rfc_get`, `rfc_search` | IETF RFCs by number or title. |
| [standards](skills/standards.md) | `standards_search` | IEEE/SAE/NIST/ISO metadata via Crossref. |
| [arxiv](skills/arxiv.md) | `arxiv_search`, `arxiv_get` | arXiv papers (PDF URLs feed `read_pdf`). |
| [huggingface](skills/huggingface.md) | `hf_search`, `hf_model` | Hugging Face Hub models/datasets. |
| [wikipedia](skills/wikipedia.md) | `wikipedia_search`, `wikipedia_summary` | Wikipedia search + article text. |
| [news](skills/news.md) | `news_feed` | Recent items from any RSS/Atom feed (or a built-in shorthand). |
| [kernel](skills/kernel.md) | `kernel_releases` | Current Linux kernel releases. |
| [github](skills/github.md) | `github_releases`, `github_user`, `github_repo` | GitHub release notes / profile / repo metadata. |

## Containers & cloud-native data (keyless)

| Skill | Tools | What |
| --- | --- | --- |
| [oci](skills/oci.md) | `docker_search`, `docker_image`, `docker_tags`, `oci_tags`, `oci_manifest` | Docker Hub + any OCI registry (tags, manifests). |
| [artifacthub](skills/artifacthub.md) | `artifacthub_search` | Helm charts / Operators / krew / policies. |

## Local system control

| Skill | Default | Tools | What |
| --- | --- | --- | --- |
| [docker](skills/docker.md) | on `[docker]` | `docker_ps`/`images`/`inspect`/`logs`/`info`/`pull`/`run`/`start`/`build` + **destructive** `stop`/`remove`/`exec`/`rmi` | Control the local Docker daemon (Engine API). |
| [kubernetes](skills/kubernetes.md) | on `[kubernetes]` | `k8s_contexts`/`get`/`describe`/`logs`/`apply`/`scale` + **destructive** `k8s_delete` | Talk to a cluster via kubeconfig (kube-rs). |
| [filesystem](skills/filesystem.md) | **off** `[filesystem]` | `fs_read`/`list`/`stat`/`find`/`write`/`edit`/`mkdir` + **destructive** `delete`/`move` | Read/edit files, confined to `roots`. |
| [shell](skills/shell.md) | **off** `[shell]` | `shell_run` | Run a command (allowlist or unrestricted). |
| [git](skills/git.md) | on `[git]` | `git_run` | Run git in a repo (destructive subcommands guarded). |
| [ffmpeg](skills/ffmpeg.md) | **off** `[ffmpeg]` | `ffmpeg_probe`, `ffmpeg_convert` | Probe/convert local media (paths confined to roots; convert guarded). |
| [spreadsheet](skills/spreadsheet.md) | **off** `[spreadsheet]` | `sheet_read`, `sheet_query`, `sheet_write` | Read/query/write CSV & XLSX (paths confined to roots; write guarded). |
| [sysinfo](skills/sysinfo.md) | on `[sysinfo]` | `system_info`, `system_disks`, `system_gpu` | Host/CPU/memory/disk + NVIDIA GPU (read-only). |
| [databases](skills/databases.md) | off (until `[databases.<id>]`) | `db_list`, `db_query`, `redis_command` | Query Postgres/MySQL/Redis (writes guarded). |

## Devices (off by default)

| Skill | Tools | What |
| --- | --- | --- |
| [serial](skills/serial.md) | `serial_ports`, `serial_send`, `serial_read` | Raw serial-device I/O (`serial_send` guarded). |
| [printer](skills/printer.md) | `printer_list`, `printer_print` | OS printing (CUPS / Windows; `printer_print` guarded). |

## Caching & storage

| Skill | Tools | What |
| --- | --- | --- |
| [store](skills/store.md) | `cache_status`, `store_fetch`, `store_get`, `store_list`, `store_purge` | On-disk file store (`[store]`, off by default) + cache stats; shared over the [hivemind](hivemind.md). |

## Utilities (local; translate/currency keyless)

| Skill | Tools | What |
| --- | --- | --- |
| [datetime](skills/datetime.md) | `datetime`, `date_diff`, `time_convert` | The model's "now" + timezone math. |
| [translate](skills/translate.md) | `translate`, `detect_language` | Google Translate (keyless). |
| [data](skills/data.md) | `json_query`, `json_format`, `yaml_to_json`, `json_to_yaml` | Parse/convert JSON & YAML. |
| [regex](skills/regex.md) | `regex_search`, `regex_replace` | Match/substitute with Rust regex. |
| [math](skills/math.md) | `math_eval`, `math_solve`, `geo_distance`, `geo_azimuth`, `wave_frequency` | Arithmetic/algebra, geo distance/bearing, wave conversions. |
| [forecast](skills/forecast.md) | `forecast` | Time-series forecasting (Holt / Holt-Winters), local. |
| [units](skills/units.md) | `convert_units` | Unit conversion across many dimensions. |

## Finance & markets (keyless)

| Skill | Tools | What |
| --- | --- | --- |
| [finance](skills/finance.md) | `compound_interest`, `loan_payment`, `currency_convert` | Interest/loan math + keyless currency conversion (ECB). |
| [stocks](skills/stocks.md) | `stock_quote`, `yahoo_quote`, `yahoo_history`, `yahoo_search` | Delayed stock/index/FX/crypto quotes, OHLC history & symbol search (keyless Stooq + Yahoo Finance). |

## Space & astronomy (keyless)

| Skill | Tools | What |
| --- | --- | --- |
| [nasa](skills/nasa.md) | `nasa_apod`, `nasa_neo`, `nasa_mars_photos` | NASA open data (DEMO_KEY; optional `[nasa].key`). |
| [satellite](skills/satellite.md) | `sat_tle`, `sat_position`, `sat_observe` | SGP4 orbit propagation: sub-point + observer look-angles. |

## Introspection

| Skill | Tools | What |
| --- | --- | --- |
| [meta](skills/meta.md) | `list_providers`, `hive_status`, `hive_peers`, `hive_seeds` | Active providers; the hivemind graph, hop distances, and seed ratios. |
