# lodestone-mcp

[![CI](https://github.com/elyerinfox/lodestone-mcp/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/elyerinfox/lodestone-mcp/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/elyerinfox/lodestone-mcp?label=release&color=brightgreen)](https://github.com/elyerinfox/lodestone-mcp/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.74%2B-orange?logo=rust)](https://www.rust-lang.org)
[![MCP](https://img.shields.io/badge/MCP-Streamable%20HTTP-7c3aed)](https://modelcontextprotocol.io)
[![Tools](https://img.shields.io/badge/tools-%7E400-success)](docs/tools.md)
[![Skills](https://img.shields.io/badge/skill%20families-%7E85-success)](docs/skills.md)

A **keyless-by-default, self-hosted [MCP](https://modelcontextprotocol.io) server**
that gives a local LLM a broad, composable toolkit — **search and retrieve** the open
web and developer ecosystem, **operate** the machine it runs on (Docker, Kubernetes,
files, shell, git, databases, serial/printers), and **compute** over real data
(math, geo, finance, units, dates, JSON/YAML/regex, NASA/space, markets) — all
without signing up for, paying for, or managing API keys.

It scrapes search engines and reads public, keyless endpoints instead of calling
paid, key-gated APIs, and talks to local daemons/devices directly. Built for local
runners like **LM Studio**, Ollama front-ends, or any Streamable-HTTP MCP client.
Written in Rust on the official [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk)
SDK; compiles to a single binary.

> **"Keyless by default" — what that means.** Everything works with **zero**
> accounts or keys. A few sources can *optionally* use a credential to unlock or
> improve them — the keyed web engines `brave`/`google_cse`, a GitHub token, a
> StackExchange key, a NASA key, a database URL. Each is **strictly optional and off
> unless you supply it**; none is ever required, and credentials are never logged or
> committed.

## Why "lodestone"?

It started as a small "search the web, retrieve code & docs" helper and kept
growing — into web/code/docs/Q&A search, page/PDF/file/archive retrieval, GitHub &
container lookups, local Docker/Kubernetes/filesystem/shell/git/database control,
host & GPU info, math/geo/finance/units/date/translation utilities, NASA/markets/
satellite data, and an opt-in peer-to-peer cache. That sprawl is the point.

This project was born out of frustration: getting a local model to actually *do*
things meant gluing together a dozen single-purpose tools, each with its own
ecosystem, install dance, auth, and quirks, just to assemble a workable toolkit.
The need it answers is for one **monolithic** solution — broad enough to cover the
surface area, yet **intelligent enough not to become a burden** itself. Keyless by
default, gated, and safe-by-construction, so adopting it costs a config line, not a
maintenance project.

The name fits. A **lodestone** is a naturally magnetized piece of magnetite — the
original compass, the very stone early navigators used to find north. That is what
this aims to be for a model: a single point that **draws scattered capabilities
together** and **orients** the model toward the right tool for the task at hand.
One stone, many bearings.

## What it is

- **A keyless toolkit for a local model** — one MCP server exposing **~400
  small, composable [tools](docs/tools.md)** organized into ~85
  [skill families](docs/skills.md), each independently gateable.
- **Search _and_ retrieve.** Finding a link is half the job; reading the page, file,
  PDF, or answer is the other half. Retrieval is first-class.
- **Local-system aware.** Beyond the web, it can inspect and operate the host:
  containers, clusters, files, processes, git repos, databases, devices, GPU.
- **Safe by construction.** Destructive actions never fire unguarded (a confirm-token
  handshake), dangerous families are off by default, and credentials stay optional.
- **A single binary you run yourself.** No SaaS, no account, offline-friendly.

## What it isn't

- **Not a hosted/keyed search API** — keyless by default; keyed providers are
  optional add-ons.
- **Not a large-scale crawler** — rendering is single-page, on demand.
- **Not an agent framework** — it's the *tools*; your MCP host/model is the agent.
- **Not a guaranteed-stable data source** — scraping is best-effort and degrades to
  fallbacks / the web archive. See the [honest limitations](docs/comparison.md#honest-limitations).

## Why this shape

Ask a plain LLM "what's the kinetic energy of a 1500 kg car at 60 mph?" and it
has to do three different jobs at once, all from inside its own weights:
remember the formula, convert the units, and run the multiplication. If any one
step goes wrong, the answer still *sounds* right. That's the hallucination
problem in a sentence.

Lodestone splits the work. The fact lookup goes to a real source (`physics_formula`
knows ½mv²; `kernel_releases` knows the current Linux release; `arxiv_get`
returns the actual abstract). The math goes to a real engine (`convert_units`
does the mph→m/s; the formula itself does the energy). The LLM stays in the
role it's reliable at — *picking which tool to call*. When a tool can't answer,
it says so out loud (404, no result, schema mismatch) instead of being silently
wrong.

Web search alone doesn't fix this — the LLM is still the one reading and
summarizing pages, and modern search results are increasingly SEO-shaped
([Bevendorff et al., *Is Google Getting Worse?*, ECIR
2024](https://arxiv.org/abs/2401.01860)). The bet is that a constellation
of small, boring, verifiable tools beats one big model trying to be
everything — the "compound AI systems" thesis ([Zaharia et al.,
*The Shift from Models to Compound AI Systems*, BAIR
2024](https://bair.berkeley.edu/blog/2024/02/18/compound-ai-systems/))
— and it's borne out empirically: [Toolformer (Schick et al., NeurIPS
2023)](https://arxiv.org/abs/2302.04761) and [WebGPT (Nakano et al.,
2021)](https://arxiv.org/abs/2112.09332) both showed tool-augmented
smaller models outperforming much larger un-tooled ones on the tasks
they target.

## How it works

Lodestone speaks MCP over **Streamable HTTP** at `/mcp`. Search sources are
`SearchProvider`s grouped by *kind* (`web`/`code`/`qa`/`docs`) and combined by a
**strategy** (`fallback` or concurrent `aggregate` + re-rank); a per-call **`render`**
flag routes scraping through a shared headless Chrome. Everything else is a
self-contained **skill** module. Results and fetched files are cached (in-memory,
optionally Redis, and an on-disk file store). Adding a capability means adding a skill
or a provider, never editing `main.rs` — see [CONTRIBUTING.md](CONTRIBUTING.md) and
the [golden rules](docs/golden-rules.md).

## What it enables

Because the skills compose, a model can chain them into real, multi-step work. The
tree below is scoped by domain → sub-field → the concrete capability and tools.

### Academia & research

- **Biomedicine & life sciences** — search the literature with `pubmed_search` and
  read an abstract with `pubmed_summary`; reach the rest of NCBI with `ncbi_search` /
  `ncbi_summary`: **genetics** (`db=gene`, `clinvar`, `snp`), **proteomics/genomics**
  (`protein`, `nucleotide`, `assembly`, `genome`), **taxonomy** (`taxonomy`), and
  full-text via PubMed Central (`pmc`). e.g. "BRCA1 variants linked to breast cancer"
  → `pubmed_search` → `ncbi_search db=clinvar`.
- **Physics, math & CS** — find a preprint with `arxiv_search` and read the free PDF
  with `read_pdf` (shared across your [constellation](docs/constellation.md), so you
  don't re-download); evaluate/solve expressions (`arithmetic_eval` / `algebra_solve`)
and plug into ~85 named formulas across fields (`physics_formula`, `geometry_formula`,
`trig_formula`, …, plus `physical_constant`); pull a
  model/dataset card from **Hugging Face** (`hf_model` / `hf_model_search` / `hf_dataset_search`).
- **Engineering & standards** — look up an **IETF RFC** (`rfc_get` / `rfc_search`)
  or an **IEEE / SAE / NIST / ISO** standard (`standards_search`, with DOI links and
  free NIST full text via `read_pdf`); unit conversions across dimensions
  (`convert_units`).
- **General reference** — `wikipedia_search` / `wikipedia_summary`, plus
  `web_search` → `fetch_page` / `render_page` for anything else (with a Wayback
  fallback, `wayback_fetch`).

### Astronomy & aerospace

- **Orbital tracking** — "when is the ISS next over Berlin?" → `sat_tle "ISS"` (fetch
  the current TLE from CelesTrak) → `sat_observe` from your coordinates for
  **azimuth / elevation / range**, or `sat_position` for the live **ground sub-point**
  (lat/lon/alt/speed) via SGP4 propagation.
- **NASA open data** — **near-Earth objects** with miss-distance/velocity/hazard
  flags (`nasa_neo`), and **Mars-rover**
  imagery (`nasa_mars_photos`).
- **Radio & signals** — convert **frequency ↔ wavelength ↔ period** (`wave_frequency`,
  e.g. antenna sizing, Doppler); with hardware, scan the RF spectrum (`sdr_scan`).
- **Geospatial** — great-circle **distance** and initial **bearing/azimuth** between
  coordinates (`geo_distance` / `geo_azimuth`) — ground stations, flight legs, siting.
- **Trajectory mechanics** — projectile-with-drag RK4 (`traj_projectile_drag`,
  variable wind + air density), **Hohmann** transfer Δv and transfer time
  (`traj_hohmann`), and **Sutton-Graves** stagnation-point reentry heating
  (`traj_reentry_heating`).
- **Open data feeds** — live aircraft state vectors from **OpenSky**
  (`open_data_opensky_states`), USGS earthquake feeds
  (`open_data_usgs_earthquakes`), and NOAA SWPC real-time solar wind
  (`open_data_swpc_solar_wind`).

### Geodesy & navigation

A pure-Rust ellipsoidal toolkit for siting, mapping, and aiding receivers.

- **Coordinate systems** — full WGS84 suite via `geographiclib`: Vincenty
  inverse/direct geodesics (`geo_vincenty_inverse` / `geo_vincenty_direct`),
  great-circle polyline densify (`geo_great_circle_polyline`), cross-track
  distance (`geo_cross_track`), ellipsoidal polygon area
  (`geo_polygon_area_geodesic`), UTM forward/inverse (`geo_utm_from_latlon`,
  `geo_latlon_from_utm`), MGRS forward/inverse (`geo_mgrs_from_latlon`,
  `geo_latlon_from_mgrs`), ECEF ↔ geodetic
  (`geo_ecef_from_latlon` / `geo_latlon_from_ecef`), and 7-parameter Helmert
  datum transform (`geo_helmert`).
- **GNSS aiding** — DOP from satellite line-of-sight unit vectors (`nav_dop`,
  PDOP / HDOP / VDOP / TDOP / GDOP), Klobuchar ionospheric delay
  (`nav_klobuchar`), Saastamoinen tropospheric delay (`nav_saastamoinen`),
  ECEF → local ENU (`nav_ecef_to_enu`), and an IMU drift error model
  (`nav_imu_drift`) combining angle random walk, bias instability, and
  scale-factor RSS.
- **Format converters** — NMEA-0183 sentence decode with XOR checksum
  verification (`convert_nmea_decode`, GPGGA/GPRMC/GPGSA/GPGSV/GPVTG),
  Cursor-on-Target XML emit for TAK pipelines (`convert_cot_encode`), and
  GeoJSON → WKT (`convert_geojson_to_wkt`).
- **Earth models** — Greenwich / local mean sidereal time
  (`earth_sidereal_time`, Meeus 12.4) and a centred-dipole magnetic
  declination (`earth_magnetic_declination`) for compass corrections.

### RF, radar & signal processing

- **Path-loss models** — two-ray plane-earth (`rf_two_ray_path_loss`),
  Okumura-Hata (`rf_hata_path_loss`, 150–1500 MHz), COST-231-Hata
  (`rf_cost231_path_loss`, 1500–2000 MHz), Egli (`rf_egli_path_loss`),
  ITU-R P.676 atmospheric absorption (`rf_itu_p676_absorption`), and ITU-R
  P.838 rain attenuation (`rf_itu_p838_rain`).
- **Link physics** — Doppler shift (`rf_doppler_shift`), polarization
  mismatch (`rf_polarization_loss`, linear ⊕ circular), Fresnel-zone radius
  (`rf_fresnel_zone_radius`), knife-edge diffraction
  (`rf_knife_edge_diffraction`, Lee J(v)), and a full Friis link budget with
  kTBF system-noise floor (`rf_friis_with_noise`).
- **Radar equation family** — mono- and bistatic ranges (`radar_monostatic`,
  `radar_bistatic`), coherent / non-coherent integration gain
  (`radar_integration_gain`), pulse compression (`radar_pulse_compression_gain`),
  CA / OS CFAR thresholds (`radar_cfar_threshold`), Rayleigh / Weibull /
  K-distribution clutter (`radar_clutter_threshold`), and radar Doppler
  (`radar_doppler_shift`).
- **DSP extensions** — beyond the basic `signal_fft` family, the new tools
  cover STFT **spectrogram** (`signal_spectrogram`), FFT **cross-correlation**
  with peak-lag (`signal_cross_correlation`), the **Hilbert transform**
  (`signal_hilbert`, analytic signal + instantaneous frequency), real
  **cepstrum** (`signal_cepstrum`), **BER curves** for BPSK / QPSK / M-PSK /
  M-QAM / FSK over AWGN or Rayleigh (`signal_ber_curve`), and **IQ demod**
  (`signal_iq_demod`).
- **Estimation & tracking** — single-step linear Kalman filter
  (`track_kalman_step`, returns NIS for chi-squared gating), Hungarian /
  Kuhn-Munkres assignment (`track_hungarian`), and 2-D RANSAC line fit
  (`track_ransac_line`).
- **Acoustic & underwater** — Mackenzie 9-term sound speed in seawater
  (`acoustic_sound_speed_water`), air sound speed (`acoustic_sound_speed_air`),
  Snell's-law refraction (`acoustic_snell`), Thorp absorption + spherical /
  cylindrical transmission loss (`acoustic_transmission_loss`), and the full
  sonar equation (`acoustic_sonar_equation`).

### Maths, control & cryptography

- **Linear algebra** — `linalg_solve` (LU), `linalg_lstsq` (least squares),
  `linalg_svd`, `linalg_eigen`, `linalg_qr`, `linalg_inv`, `linalg_det`,
  `linalg_rank`, `linalg_norm`, `linalg_matmul`. Pure-Rust via `nalgebra`.
- **Quaternions & attitude** — Euler ↔ quaternion (`quat_from_euler`,
  `quat_to_euler`), compose / rotate / conjugate / normalize / slerp, plus
  an Euler → DCM helper (`frame_dcm_from_euler`). Hamilton convention.
- **ODE integration** — classical fourth-order Runge-Kutta (`ode_rk4`); the
  per-state right-hand-side is supplied as a `meval` expression referring to
  `t` and `y0`, `y1`, …
- **Information theory & coding** — Shannon-Hartley capacity
  (`it_shannon_capacity`), Rényi-generalized entropy (`it_entropy`), KL
  (`it_kl_divergence`) and JS (`it_js_divergence`) divergence, mutual
  information from a joint distribution (`it_mutual_information`), Hamming
  distance (`code_hamming_distance`), CRC (`code_crc`, 8/16/32/64), Reed-Solomon
  encode (`code_rs_encode`), and a K=7 rate-½ convolutional encoder
  (`code_convolutional_encode`).
- **Crypto primitives as math tools** — Miller-Rabin primality
  (`crypto_miller_rabin`), big-integer `crypto_modexp` / `crypto_mod_inverse`,
  Chinese Remainder Theorem (`crypto_crt`), HKDF (`crypto_hkdf`), PBKDF2
  (`crypto_pbkdf2`), Argon2id (`crypto_argon2`), HMAC over
  SHA-1/256/384/512 (`crypto_hmac`), and `crypto_jwt_decode`
  (decode-without-verification, educational). **Not** a production TLS / KMS
  surface — these are exposed as math/research tools, not as a secrets vault.
- **Optimization & operations research** — TSP via nearest-neighbour + 2-opt
  (`opt_tsp_2opt`) and shortest path on a directed weighted graph
  (`opt_shortest_path`, Dijkstra).
- **Atmospherics** — US-Standard-Atmosphere-1976 ISA (`atm_isa`), density
  altitude (`atm_density_altitude`), Magnus dewpoint (`atm_dewpoint`), Stull
  WBGT (`atm_wbgt`), and a live NOAA SWPC planetary K-index
  (`atm_space_weather_kp`) for HF-prop / aurora context.
- **Specialist visualizations** — polar antenna pattern (`chart_polar`),
  Smith chart (`chart_smith`), spectrogram waterfall (`chart_waterfall`),
  compass / wind rose (`chart_compass_rose`), sky plot for satellite az/el
  (`chart_skyplot`), and a 2-D density heatmap (`chart_density_map`). All SVG,
  no extra deps beyond the existing `chart_*` family.

### Chemistry & life sciences

Validated, citation-backed: every formula lists its source paper or
standards body in the per-tool description (see
**[skills/chemistry.md](docs/skills/chemistry.md)**,
**[skills/biology.md](docs/skills/biology.md)**,
**[skills/bio_data.md](docs/skills/bio_data.md)**).

- **Chemistry primitives** — IUPAC CIAAW 2021 atomic weights
  (`chem_periodic_table`), formula molar mass with parentheses + hydrate
  parsing (`chem_molar_mass`), Hill-order normalization
  (`chem_formula_hill`), **exact integer equation balancing** via
  fraction-free Gauss-Jordan + LCM/GCD (`chem_balance_equation` — no
  SVD round-off), pH for strong/weak acid-base (`chem_ph`),
  Henderson-Hasselbalch buffer (`chem_buffer`), the ideal gas law
  (`chem_ideal_gas`), M₁V₁ = M₂V₂ dilution (`chem_dilution`), ΔG = ΔH −
  TΔS (`chem_gibbs`), and first-order radioactive decay
  (`chem_radioactive_decay`).
- **Bioinformatics primitives** — DNA/RNA/protein operations using the
  NCBI standard genetic code (`bio_transcribe`, `bio_translate`,
  `bio_dna_complement`, `bio_gc_content`, `bio_codon_lookup`,
  `bio_orf_finder`), monoisotopic peptide MW via Unimod/Expasy
  reference masses (`bio_protein_mw`), Wallace + basic Marmur primer
  Tm (`bio_pcr_tm`), Needleman-Wunsch + Smith-Waterman alignment
  (`bio_align_global`, `bio_align_local`), Michaelis-Menten enzyme
  kinetics (`bio_michaelis_menten`), Hardy-Weinberg equilibrium
  (`bio_hardy_weinberg`).
- **Live data feeds** — keyless REST fetches from UniProt
  (`bio_uniprot_get`), RCSB Protein Data Bank (`bio_pdb_get`), and
  Ensembl (`bio_ensembl_lookup`).

### Nuclear physics

Vendored AME2020 / NUBASE2020 nuclide subset (`nuke_nuclide_lookup`),
Bethe-Weizsäcker semi-empirical mass formula with the Krane coefficient
set (`nuke_binding_energy`), atomic-mass-unit ↔ MeV via CODATA 2022
(`nuke_unit_convert`), reaction Q-values from atomic masses
(`nuke_q_value`), first-order decay law (`nuke_decay_law`), and the
closed-form Bateman two-step chain with proper handling of the
λ_A = λ_B limit (`nuke_decay_chain`). Full source list in
**[skills/nuclear.md](docs/skills/nuclear.md)**.

### Radiation protection

Gy ↔ rad / Sv ↔ rem / R ↔ air-kerma conversions (`rad_units`),
exponential attenuation with HVL / TVL (`rad_attenuation`),
shielding-thickness calculator driven by **NIST XCOM mass-attenuation
coefficients** for Pb / concrete / steel / water / Al
(`rad_shielding_thickness`), inverse-square distance dose
(`rad_inverse_square`), idealized point-source dose rate from a
vendored Γ table (`rad_dose_rate`), the full ICRP 103 piecewise
neutron-w_R equivalent dose (`rad_equivalent_dose`), biokinetic
effective half-life (`rad_effective_half_life`), the classic ALARA
time / distance / shielding triad (`rad_alara`), and side-by-side ICRP
103 vs US 10 CFR 20 occupational limits (`rad_occupational_limits`).
Plus a vendored radioisotope reference table covering industrial,
research, calibration, and medical sources (Co-60, Cs-137, Ir-192,
Am-241, Ra-223, Mo-99, Tc-99m, I-131, F-18, Ga-68, Lu-177, Y-90, …)
via `rad_isotope_lookup`. Sources in
**[skills/radiology.md](docs/skills/radiology.md)**.

### Machining

Cutting speed → RPM (`mach_cutting_speed`), feed rate
(`mach_feed_rate`), material removal rate (`mach_mrr_milling`),
Sandvik **Kienzle cutting power** with vendored k_c1 / m_c per
material group (`mach_cutting_power`), theoretical surface finish in
turning (`mach_surface_finish_turning`), Shigley **beam deflection**
for the four common cases (`mach_beam_deflection`), area moment of
inertia for rectangle / round (`mach_section_inertia`), axial
stress / strain (`mach_stress_strain`), Shigley-table bolt torque
(`mach_bolt_torque`), vendored **UNC + ISO metric coarse** thread +
tap-drill table (`mach_thread_spec`), MatWeb / ASM **material
properties** (`mach_material`), and ASTM E140 **hardness conversion**
(`mach_hardness_convert`). Sources in
**[skills/machinist.md](docs/skills/machinist.md)**.

### CNC / OpenSCAD

Emit portable RS-274/NGC G-code for single drilled holes
(`gcode_drill_hole`) and circular bolt patterns
(`gcode_bolt_pattern`); parse and summarize any G-code program
(`gcode_parse_summary`) — command counts, modal state, bounding box,
axis travel. Generate OpenSCAD source for primitives (`scad_box`,
`scad_cylinder`, `scad_sphere`) and the canonical
"flange with N bolt holes on a PCD" idiom (`scad_flange`). Targets
the LinuxCNC / Grbl / Marlin intersection so output is portable.
Sources in **[skills/cnc.md](docs/skills/cnc.md)**.

### Mesh / 3-D interchange

Probe an STL file (binary or ASCII), returning triangle count,
axis-aligned bounding box, surface area, and centroid
(`interchange_stl_info`). Foundation for richer MAVLink / NetCDF /
DICOM surfaces in a follow-up release.

### Software & infrastructure

- **Development** — `code_search` across GitHub/GitLab/Gitea → `fetch_repo_file` to
  read the exact source; `docs_search` across crates.io / npm / MDN and framework
  docs; `github_releases` to summarize what changed between versions.
- **DevOps & SRE** — triage a box without a shell: `docker_ps` → `docker_logs`,
  `k8s_get` → `k8s_logs` → `k8s_scale`, `system_info` / `system_disks` /
  `system_gpu_nvidia` / `system_gpu_amd` / `system_gpu_intel`,
  `git_run`, and a guarded `db_query` / `redis_command` to inspect state. Destructive
  steps (delete/remove/exec) pause for confirmation.
- **Containers & registries** — image/tag/manifest lookups across Docker Hub, any OCI
  registry, and Artifact Hub (`docker_search`, `oci_tags`, `oci_manifest`,
  `artifacthub_search`).

### Markets, data & media

- **Finance & markets** — live FX (`currency_convert`, ECB), interest/loan math
  (`compound_interest` / `loan_payment`), and delayed equity/index/crypto quotes,
  history, and symbol search (`stock_quote`, `yahoo_quote` / `yahoo_history` /
  `yahoo_search`).
- **Time series & news** — forecast a numeric series (`forecast`, Holt-Winters) and
  follow any RSS/Atom feed (`news_feed`).
- **Data & files** — JSON/YAML/`regex` wrangling, CSV/XLSX read-query-write
  (`sheet_*`), media probe/convert (`ffmpeg_*`), PDFs (`read_pdf` / `webpage_to_pdf`).

The full, exhaustive lists: **[skills](docs/skills.md)** · **[tools](docs/tools.md)**
· **[providers](docs/providers.md)**.

## Operating it

Beyond what the model *does*, a few things govern how lodestone *runs* (all opt-in /
defaulted sensibly):

- **Safety & gating** — every tool is independently gateable (`[tools]`); dangerous
  local-system families are off by default; destructive actions never fire unguarded
  (a confirm-token handshake). Optional bearer auth on `/mcp`.
- **Resilience** — composite re-ranking, per-provider timeout + circuit breaker, and
  multi-route egress (proxy / headless browser) so one blocked source can't stall a
  search.
- **Caching** — search and retrieval results cache in-memory (optionally Redis), plus
  an on-disk file store for fetched bytes.
- **Scale out** — run several instances as a [constellation](docs/constellation.md)
  that serves each other's cached results/PDFs (hash-only on the wire), optionally
  linked across networks by a [galaxy](docs/constellation.md#galaxy--linking-constellations)
  broker. Long work can run in the background (`search_async` → `tasks_result`).

## Quick start

Requires a recent Rust toolchain. Node is **not** needed for the MCP server —
the dashboard ships as a separate service (own image, own container).

```sh
cargo run --bin lodestone-mcp        # MCP server only, no dashboard.
```

The crate ships **two binaries** — `lodestone-mcp` (the MCP server) and
`lodestone-galaxy` (the optional rendezvous broker that links separate
constellations across networks — see
**[docs/constellation.md](docs/constellation.md#galaxy--linking-constellations)**).
Bare `cargo run` is ambiguous; always pass `--bin lodestone-mcp` to launch
the server.

Listens on `http://127.0.0.1:8000/mcp` (and `GET /health` returns `ok`). Keyless out
of the box. The headless browser is always compiled in; the `google` engine,
per-call `render=true`, and the `browser_*` tools additionally need a local
**Chrome/Chromium** at runtime.

Endpoints the binary exposes: `/mcp`, `/ws/status`, `/api/settings/*`,
`/constellation/*`, `/api/memory/graph`, `/health`. The dashboard SPA is a
**separate service** — see "Docker" below or
**[docs/building.md](docs/building.md)** for the dev/build workflow. Every
crate and npm package the project pulls in, by purpose, is in
**[docs/dependencies.md](docs/dependencies.md)**.

### Wiring it into an MCP host

Lodestone speaks **MCP over Streamable HTTP** at `/mcp`. Any compliant
MCP host can connect — point yours at `http://127.0.0.1:8000/mcp` and
go. Per-host config snippets (LM Studio, Claude Code, Claude Desktop,
Continue, Cline, Cursor, Goose, Zed, and the canonical stdio-bridge
pattern for any other host) live in
**[docs/setup.md](docs/setup.md)**. The canonical client config shape
is also in **[`mcp.example.json`](mcp.example.json)** at the repo root.

**Docker** — two services, two images, one command:

```sh
docker compose up --build
# → MCP server   http://localhost:8000   (lodestone-mcp,   built from ./Dockerfile)
# → Dashboard    http://localhost:8001   (lodestone-dashboard, built from frontend/Dockerfile)
```

Skip the dashboard with `docker compose up --build lodestone` — the MCP
binary serves the WebSocket feed and HTTP APIs regardless. See
[docs/building.md](docs/building.md) for the dev workflow (Nuxt HMR).

## Configuration

The repo ships a working, keyless config in [`config/`](config/) (granular files,
deep-merged); override it with a gitignored `lodestone.toml` or `LODESTONE_*` env
vars. Local-system families (`[filesystem]`, `[shell]`, `[serial]`, `[printer]`,
`[store]`, `[databases.*]`) are **off by default**. Full schema, env vars, auth,
strategies, caching, forges/doc-sites: **[docs/configuration.md](docs/configuration.md)**.

## Documentation

| Doc | What's in it |
| --- | --- |
| [building.md](docs/building.md) | Backend-only / backend+dashboard / Docker / dev workflow + common build issues. |
| [dependencies.md](docs/dependencies.md) | Every crate and npm package, by purpose. License + audit notes. |
| [skills.md](docs/skills.md) | Every skill family, grouped, with a page each. |
| [tools.md](docs/tools.md) | Every tool, its arguments, and purpose. |
| [providers.md](docs/providers.md) | Every search provider, by family, with a page each. |
| [configuration.md](docs/configuration.md) | Full config schema, env vars, auth, strategies, caching. |
| [ranking.md](docs/ranking.md) | The composite ranker: signals, formulas, tuning. |
| [containers.md](docs/containers.md) | Docker Hub / OCI / Artifact Hub lookups. |
| [constellation.md](docs/constellation.md) | The opt-in peer-to-peer layer (results + blob sharing). |
| [memory.md](docs/memory.md) | Persistent memory: notes, recorded solutions (with a typed graph), synonyms, and the **intrinsic recall** that fires on every query-bearing tool. |
| [golden-rules.md](docs/golden-rules.md) | The project's invariants. |
| [comparison.md](docs/comparison.md) | How Lodestone compares; limitations. |
| [CONTRIBUTING.md](CONTRIBUTING.md) | Architecture and how to add a skill/provider. |

## Constellation — share the load, be a good neighbor

Lodestone reaches the open web by scraping search engines and fetching from
rate-limited sources (arXiv, IETF, registries, …). Those limits are almost always
enforced **per IP**, not per user — so when several people share an uplink (an
office, a lab, a campus, a VPN, a household behind one NAT), every redundant scrape
you make spends a budget your neighbors also draw on. Hammer DuckDuckGo from a
shared egress and *everyone* behind that address starts seeing tarpits and 403s,
not just you. The cost of one greedy node is paid by the whole network.

The opt-in [**constellation**](docs/constellation.md) turns that dynamic around. When you
enable it, your instance first asks its peers whether one of them has *already*
fetched a query or file before it goes to the open web:

- **Fewer requests per IP.** A result or PDF that any one node retrieves is served
  to the others, so the group hits the rate-limited source once instead of N times.
  You stop competing with your colleagues for the same shrinking budget — and stop
  being the reason their searches start failing.
- **You give as much as you get.** Every node both consults and serves: the cache
  you fill from your own work softens the next person's load, and theirs softens
  yours. A shared connection becomes a reason the experience gets *better* as more
  people join, not worse.
- **Privacy-preserving by design.** Only *hashes* of query keys cross the wire
  (never raw query text), responses carry only already-public web results/bytes
  (never secrets), peer data is trusted only by content-verified consensus, and the
  `/constellation` endpoints can require a shared `[network].token`. It stays strictly
  opt-in and is never a dependency — local search works with zero peers.

If you run more than one instance, or share a network with others who do, please
consider turning it on for your peers' sake: set `[network].enabled = true` (LAN
peers are found automatically over mDNS; add `[network].peers` for off-LAN nodes).
See [`config/06-network.toml`](config/06-network.toml) and
[docs/constellation.md](docs/constellation.md).

To link constellations **across** networks, an optional `[galaxy]` broker keeps a
directory of each constellation's public ingress endpoints so they can find and
talk to each other directly (it never proxies traffic). Entirely optional and off
by default — see [docs/constellation.md → Galaxy](docs/constellation.md#galaxy--linking-constellations).

## Golden rules

The project's non-negotiable invariants live in
**[docs/golden-rules.md](docs/golden-rules.md)** — read them there. The
README does not restate them so there's a single source of truth.

## Disclaimer

**No warranty.** Lodestone is provided "AS IS", without warranty of any kind, express
or implied. In no event shall the authors be liable for any claim, damages, or
liability arising from its use (this restates the MIT [LICENSE](LICENSE), which
governs).

**Use at your own risk.** Lodestone scrapes third-party sites and calls public
endpoints — you are responsible for complying with their terms and for any
rate-limiting that results. Its **local-system** tools are powerful: the Docker,
Kubernetes, filesystem, shell, git, database, serial, and printer families act on
your real machine, daemon, cluster, devices, and data. They are **gated** (the most
dangerous off by default) and **destructive actions require a confirmation step**,
and they're meant to run behind an MCP host that approves calls — review what you
enable, scope credentials/contexts narrowly, and prefer read-only or non-production
targets when in doubt. You are responsible for everything the model does through them.

## Roadmap & license

Planned work and known gaps: [TODO.md](TODO.md). Licensed **MIT** (see
[LICENSE](LICENSE)).

## Supporting the project

Lodestone is free, open-source, and keyless by design — there is nothing to buy and
no account to create. It is developed and maintained in spare time.

If lodestone has helped you finally get genuine, practical use out of running local
LLMs, please consider chipping in a few dollars toward its continued development and
upkeep via [GitHub Sponsors](https://github.com/sponsors/elyerinfox). Contributions
are entirely voluntary and never gate any feature — every capability remains
available to everyone, sponsor or not. Non-financial support is just as valued:
starring the repo, filing thoughtful issues, and contributing fixes or new
skills/providers all help the project thrive.
