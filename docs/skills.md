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
  `[serial]`, `[printer]`, `[store]`, `[databases]`, `[network]`). Filesystem,
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
| [pubmed](skills/pubmed.md) | `pubmed_search`, `pubmed_summary`, `ncbi_search`, `ncbi_summary` | PubMed literature/abstracts + any NCBI database (gene, protein, taxonomy, …) via keyless E-utilities. |
| [openaccess](skills/openaccess.md) | `unpaywall_lookup`, `openalex_search`, `openalex_work` | Find LEGAL open-access full text by DOI/search (Unpaywall + OpenAlex) → feed `read_pdf`. |
| [huggingface](skills/huggingface.md) | `hf_model_search`, `hf_dataset_search`, `hf_model` | Hugging Face Hub models/datasets. |
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
| [databases](skills/databases.md) | **off** `[databases]` | `db_query`, `redis_command` | Query Postgres/MySQL/Redis via a connection URL passed in the call (no preconfig; writes guarded). |

## Devices (off by default)

| Skill | Tools | What |
| --- | --- | --- |
| [serial](skills/serial.md) | `serial_ports`, `serial_send`, `serial_read` | Raw serial-device I/O (`serial_send` guarded). |
| [printer](skills/printer.md) | `printer_list`, `printer_print` | OS printing (CUPS / Windows; `printer_print` guarded). |
| [sdr](skills/sdr.md) | `sdr_devices`, `sdr_scan` | List SDRs + sweep the RF spectrum (RTL-SDR/HackRF; receive-only). |

## Service control (Linux, off by default `[systemd]`)

| Skill | Tools | What |
| --- | --- | --- |
| [systemd](skills/systemd.md) | `systemd_list`, `systemd_status`, `systemd_show` + **destructive** `start`/`stop`/`restart` | Linux systemd unit control. Destructive verbs guarded. |

## Runtimes (off by default)

| Skill | Tools | What |
| --- | --- | --- |
| [python](skills/python.md) | `python_run` | Run a Python script via the configured interpreter (`[python]`). Guarded. |

## Binary / signal / pcap / notebook (off by default)

A pure-Rust toolkit for inspecting binaries, signals, packet captures, and
notebooks. All read-only; paths confined to `[filesystem].roots`. The
signal-processing family pairs with `wave_*` for FFT-of-decoded-audio flows.

| Skill | Tools | What |
| --- | --- | --- |
| [binary](skills/binary.md) | `binary_info`, `binary_strings`, `binary_entropy`, `binary_hexdump` | ELF / PE / Mach-O probe (via `object`), strings, Shannon entropy, hexdump. |
| [signal](skills/signal.md) | `signal_fft`, `signal_dominant_frequencies`, `signal_rms`, `signal_window` | FFT (rustfft, runtime SIMD), dominant frequencies, RMS, windowing (Hann/Hamming/Blackman/rect). |
| [wave](skills/wave.md) | `wave_info`, `wave_samples` | Read a `.wav` file (via `hound`). |
| [pcap](skills/pcap.md) | `pcap_info`, `pcap_packets` | Read a `.pcap` file (pure Rust). |
| [disasm](skills/disasm.md) | `disasm_x86_hex`, `disasm_x86_file` | x86 / x64 disassembly (via `iced-x86`, NASM flavor). |
| [notebook](skills/notebook.md) | `notebook_info`, `notebook_cells` | Read a Jupyter `.ipynb` file. |

## Image / chart / HTML rendering (`[image]`, `[chart]`, `[html]` — on by default)

| Skill | Tools | What |
| --- | --- | --- |
| [image](skills/image.md) | `image_info`, `image_exif`, `image_jpeg_analyze`, `image_png_analyze` | Format / dimensions / EXIF (incl. GPS) / JPEG-marker / PNG-chunk walk. Forensic divergence flags. Paths confined to roots. |
| [chart](skills/chart.md) | `chart_line`, `chart_bar`, `chart_scatter`, `chart_histogram`, `chart_pie`, `chart_heatmap`, `chart_grafana`, `chart_stat`, `chart_gauge`, `chart_bar_gauge`, `chart_state_timeline`, `chart_candlestick`, `chart_sparkline`, `chart_canvas`, `chart_interactive`, `chart_mermaid` | Pure-Rust SVG charts (matplotlib equivalents + Grafana-style operational panels) + procedural canvas + interactive HTML (Chart.js / Plotly) + mermaid passthrough. |
| [html](skills/html.md) | `html_render` | Execute HTML / a URL in headless Chrome and return diagnostics: console events, JS exceptions, network failures, HTTP 4xx/5xx errors. |
| [browser_session](skills/browser_session.md) | `browser_open`, `browser_navigate`, `browser_click`, `browser_type`, `browser_wait`, `browser_extract`, `browser_eval`, `browser_screenshot`, `browser_list`, `browser_close`, `browser_persona_get`, `browser_persona_list`, `browser_persona_reset`, `browser_persona_delegate` | Long-lived headless-Chromium tabs the model drives across multiple tool calls. Sessions are ephemeral; **personas** are named long-lived warm identities (accumulate cookies — rate-limit relief). Constellation peers can ask us to host **guest sessions** under `[network.capabilities].browser`; the SSRF guard restricts those to public hosts. |

## Weather, geo & infrastructure (keyless)

| Skill | Tools | What |
| --- | --- | --- |
| [weather](skills/weather.md) | `weather_forecast`, `weather_marine`, `weather_air_quality`, `weather_historical` | Open-Meteo point queries — same NWP models Ventusky aggregates (GFS / ECMWF / ICON / …), plus marine, air quality, and ERA5 reanalysis. |
| [noaa](skills/noaa.md) | `noaa_alerts`, `noaa_forecast` | NOAA / NWS active weather alerts + point forecast (US coverage). |
| [osm](skills/osm.md) | `osm_geocode`, `osm_reverse_geocode`, `osm_overpass`, `osm_elevation`, `osm_route` | OpenStreetMap (Nominatim + Overpass + Open-Elevation + OSRM). |
| [grid](skills/grid.md) | `grid_power_plants`, `grid_transmission_lines`, `grid_substations`, `grid_data_centres`, `grid_gas_pipelines`, `grid_submarine_cables` | Critical-infrastructure layers via OpenStreetMap Overpass. |
| [peeringdb](skills/peeringdb.md) | `peeringdb_network`, `peeringdb_ix`, `peeringdb_facility`, `peeringdb_org` | Networks / IXs / facilities / orgs (peeringdb.com). |
| [fcc](skills/fcc.md) | `fcc_callsign`, `fcc_amateur_bands`, `fcc_radio_service` | US amateur callsign lookup + amateur band plan + FRS / GMRS / MURS / CB reference. |

## Finance & markets (extended)

| Skill | Tools | What |
| --- | --- | --- |
| [yahoo](skills/yahoo.md) | `yahoo_quote`, `yahoo_history`, `yahoo_search` | Yahoo Finance: delayed quote, OHLC history, symbol search. Keyless. Joins the `stocks` family. |

## Energy (keyless, optional key)

| Skill | Tools | What |
| --- | --- | --- |
| [eia](skills/eia.md) | `eia_series`, `eia_browse` | U.S. Energy Information Administration time series (electricity / NG / petroleum / coal / renewables / international). Requires free `[eia].key`. |

## Astronomy & radio (keyless, off by default)

| Skill | Tools | What |
| --- | --- | --- |
| [astro](skills/astro.md) | `astro_sun`, `astro_moon` | Sun / moon position, rise / transit / set, phase. Local compute. |
| [radio](skills/radio.md) | `radio_fspl`, `radio_link_budget`, `radio_antenna` | RF link math: free-space path loss, link budget, antenna gain ↔ effective aperture. |

## Caching & storage

| Skill | Tools | What |
| --- | --- | --- |
| [store](skills/store.md) | `cache_status`, `store_fetch`, `store_get`, `store_list`, `store_purge` | On-disk file store (`[store]`, off by default) + cache stats; shared over the [constellation](constellation.md). |
| [tasks](skills/tasks.md) | `task_run`, `task_list`, `task_status`, `task_result`, `task_cancel` | Background jobs (off by default `[tasks]`): run a search off the request path, poll for results. |
| [memory](skills/memory.md) | `remember`, `remember_fact`, `remember_solution`, `recall`, `memory_save`/`get`/`list`/`search`/`forget`, `solution_record`/`find`/`show`/`list`/`update`/`forget`/`link`/`unlink`/`graph`/`related`, `synonym_*` | Persistent memos + advisory recall of prior **solutions** across sessions (revisions tracked, fuzzy + synonym + tag matching, typed relation graph). Frictionless `remember` auto-classifies fact vs solution + auto-keys + auto-tags; symmetric `recall` merges memo + solution + phrasing hits. Intrinsic recall fires a "💡 prior solutions" + "📝 facts you noted" preamble on every query-bearing tool call. On by default `[memory]`. |

## Utilities (local; translate/currency keyless)

| Skill | Tools | What |
| --- | --- | --- |
| [datetime](skills/datetime.md) | `datetime`, `date_diff`, `time_convert` | The model's "now" + timezone math. |
| [translate](skills/translate.md) | `translate`, `detect_language` | Google Translate (keyless). |
| [data](skills/data.md) | `json_query`, `json_format`, `yaml_to_json`, `json_to_yaml` | Parse/convert JSON & YAML. |
| [regex](skills/regex.md) | `regex_search`, `regex_replace` | Match/substitute with Rust regex. |
| [forecast](skills/forecast.md) | `forecast_holt_linear`, `forecast_holt_winters` | Time-series forecasting — one tool per method (Holt linear / Holt-Winters), local. |
| [units](skills/units.md) | `convert_units` | Unit conversion across many dimensions. |

## Math & science (local, by field)

Named-formula registries (compute by id from a `{var: value}` map; discover with the
matching `*_formula_list`) plus the expression evaluator and equation solver. The
[`formula`](skills/formula.md) module is the shared engine each domain dispatches
through — no tools of its own.

| Skill | Tools | What |
| --- | --- | --- |
| [arithmetic](skills/arithmetic.md) | `arithmetic_eval` | Evaluate free-form expressions (sqrt, sin, pi, `^`, …). |
| [algebra](skills/algebra.md) | `algebra_solve`, `algebra_formula`, `algebra_formula_list` | Solve linear/quadratic equations; combinatorics (nPr, nCr, factorial, discriminant). |
| [geometry](skills/geometry.md) | `geo_distance`, `geo_azimuth`, `geometry_formula`, `geometry_formula_list` | Great-circle distance/bearing; areas, volumes, Pythagoras, Heron, law of cosines. |
| [trigonometry](skills/trigonometry.md) | `trig_formula`, `trig_formula_list` | sin/cos/tan + inverses (degrees), deg↔rad, law of sines/cosines, arc/sector. |
| [physics](skills/physics.md) | `physics_formula`, `physics_formula_list`, `physical_constant`, `wave_frequency` | ~70 physics formulas (mechanics→fluids) + SI constants + wave f/λ/T. |
| [formula](skills/formula.md) | (infrastructure) | Shared registry engine: input validation, listing, uniform response shape. Every `*_formula` / `*_formula_list` tool dispatches through it. |

## Finance & markets (keyless)

| Skill | Tools | What |
| --- | --- | --- |
| [finance](skills/finance.md) | `compound_interest`, `loan_payment`, `currency_convert` | Interest/loan math + keyless currency conversion (ECB). |
| [stocks](skills/stocks.md) | `stock_quote`, `yahoo_quote`, `yahoo_history`, `yahoo_search` | Delayed stock/index/FX/crypto quotes, OHLC history & symbol search (keyless Stooq + Yahoo Finance). |

## Space & astronomy (keyless)

| Skill | Tools | What |
| --- | --- | --- |
| [nasa](skills/nasa.md) | `nasa_neo`, `nasa_mars_photos` | NASA open data (DEMO_KEY; optional `[nasa].key`). |
| [satellite](skills/satellite.md) | `sat_tle`, `sat_position`, `sat_observe` | SGP4 orbit propagation: sub-point + observer look-angles. |

## Introspection

| Skill | Tools | What |
| --- | --- | --- |
| [meta](skills/meta.md) | `list_providers`, `constellation_status`, `constellation_peers`, `constellation_seeds`, `constellation_capabilities` | Active providers; the constellation graph, hop distances, seed ratios, and per-feature capability advertisement ("who can do X?"). |

## Infrastructure (no tools — backs other skills)

These modules don't expose tools directly; they're the shared engines /
guards that other skill modules build on top of. Each has its own doc
so other skill docs (and the security audit) can link into a single
canonical reference.

| Module | Doc | What |
| --- | --- | --- |
| `guard` | [guard.md](skills/guard.md) | Client-agnostic two-call confirmation for destructive actions. First call returns a one-time token; second call with `confirm` runs. Bypassed by `[<family>].allow_destructive`. |
| `ssrf` | [skills/ssrf.md](skills/ssrf.md) | URL guard the browser session manager applies to **guest sessions** (peer-hosted). Refuses RFC1918 / loopback / link-local / CGNAT / IPv6 ULA / local TLDs. Synchronous for literal IPs, DNS-resolving for hostnames. |
| `formula` | [formula.md](skills/formula.md) | Shared named-formula registry. Backs `algebra_formula` / `geometry_formula` / `trig_formula` / `physics_formula` so each domain's catalog can be one closure per formula instead of one tool per formula. |
