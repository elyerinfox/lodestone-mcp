# How lodestone compares

This page exists for one purpose: help you decide whether lodestone is
the right thing for what you're building, or whether one of the
neighbouring projects fits better. We don't try to win every column —
the project has a specific shape (keyless-by-default, code-aware,
MCP-native, self-hosted, single binary, ~400 tools across ~85 skill
families) and the trade-offs that come with that shape are real. They
are laid out below, in detail, in the [Honest limitations](#honest-limitations)
section.

If you want the one-paragraph summary: lodestone is for **local-model
operators who want a broad, composable toolkit without signing up for
or managing a stack of API keys**. It scrapes search engines and reads
public keyless endpoints instead of calling paid APIs; it talks to
local daemons directly; it ships as one binary; everything is gated;
destructive actions are guarded; the math/science tools cite their
sources and are audited. The cost of that shape is breadth-vs-depth
trade-offs against specialist projects, scrape brittleness on the
search path, and a long list of domain-specific approximations
documented below.

## Table of contents

- [The category](#the-category)
- [Side-by-side comparison](#side-by-side-comparison)
- [Per-tool / per-host deep dives](#per-tool--per-host-deep-dives)
- [What lodestone is **not**](#what-lodestone-is-not)
- [Honest limitations](#honest-limitations)
- [When to pick something else](#when-to-pick-something-else)

## The category

"MCP server for an LLM toolkit" is a small but growing category. There
are three rough archetypes:

1. **Single-purpose MCP servers** — one capability surfaced over MCP
   (the official `fetch` server, Brave Search MCP, GitHub MCP).
   Easy to drop in; you compose several of them in the host config.

2. **Hosted SaaS MCP services** — Tavily, Exa, Firecrawl. Hosted
   crawl/search infrastructure with an MCP front-end. Generally fast,
   well-ranked, and require an API key + usage budget.

3. **Local toolkits with MCP surfaces** — the project you're reading.
   A larger surface area (search + retrieve + code + science + system
   control + memory + constellation) bound into one server, with
   most of the surface working out of the box, keyless.

Each archetype is right for some use case. The table below
focuses on the comparison most lodestone candidates actually face:
"I want a broad set of tools my local LLM can use; what should I
self-host?"

## Side-by-side comparison

A wider, more honest table than the one we used to ship. Each "Yes" /
"No" / "Partial" carries the caveat in the next column or in the deep
dives below.

| Capability | lodestone | SearXNG (+ MCP wrapper) | Brave / Tavily / Exa MCP | Anthropic `fetch` MCP | Firecrawl | Official GitHub MCP | Continue / Cline built-in |
| --- | --- | --- | --- | --- | --- | --- | --- |
| API key required on the default path | No¹ | No | **Yes** | No | **Yes** | **Yes** (token) | host-bundled |
| MCP-native (Streamable HTTP) | **Yes** | No, needs adapter | Yes (varies) | Yes (stdio) | Yes | Yes (stdio) | n/a (in-host) |
| Multi-engine web search | 2 keyless + 2 keyed | **~200 engines** | one each | No | partial (via crawl) | No | host's web search |
| Code / forge search | **GitHub + GitLab + Codeberg + Gitea** | via engines | No | No | No | GitHub only | host's grep/git |
| Raw file / blob fetch | **Yes** (blob + raw URL + `owner/repo/path` shorthand) | No | No | partial (HTML→text) | Yes | Yes (auth-only) | host's read |
| PDF text extract | **Yes** (`read_pdf`, local + URL) | No | No | No | partial | No | host-specific |
| Headless render (JS sites) | **On demand** (`render=true` per call) | No | n/a (hosted) | No | Yes | n/a | host-specific |
| Wayback archive fallback | **Yes** (`wayback_fetch`) | No | No | No | No | No | No |
| Academic literature | arXiv, PubMed, OpenAlex, Unpaywall | via engines | No | No | No | No | No |
| Q&A network | **StackExchange family** (with answers) | via engines | No | No | No | No | No |
| Container/registry lookups | **Docker Hub + OCI + Artifact Hub** | No | No | No | No | No | No |
| RFC / IEEE / SAE / NIST | **Yes** (`rfc_*`, `standards_search`) | via engines | No | No | No | No | No |
| Local system control | **Docker, k8s, fs, shell, git, packages, systemd, dbs** | No | No | No | No | No | host's shell |
| Hardware / devices | **Serial, printer, SDR, GPS via MQTT/meshtastic** | No | No | No | No | No | No |
| Science / math tool surface | **~150 tools across 19 families** | No | No | No | No | No | No |
| Citation discipline on math tools | **Required (golden rule 12)** | n/a | n/a | n/a | n/a | n/a | n/a |
| Audit ledger of how each formula was verified | **[audit-report.md](audit-report.md)** | n/a | n/a | n/a | n/a | n/a | n/a |
| Self-hosted single binary | **Yes** | Python + Redis | No (SaaS) | Yes (Python) | No (SaaS) | Yes (Go) | n/a |
| Offline-friendly | partial² | partial | No | Yes | No | requires auth | n/a |
| Result ranking | composite (RRF + consensus + relevance + authority + diversity) | strong | strong (hosted) | n/a | strong (hosted) | n/a | n/a |
| Background tasks | **Global `background: true` flag, MCP `tasks_*`** | No | partial | No | n/a | No | n/a |
| Persistent memory + solution recall | **Yes** (`memory_*`, `solution_*`) | No | No | No | No | No | host-specific |
| Multi-instance P2P share | **constellation** (Bloom-gated, consensus-trusted) | No | No | No | No | No | No |
| Cross-network rendezvous | **galaxy** (opt-in broker) | No | No | No | No | No | No |
| Sensitive-data handling | redacted in logs + responses + constellation + echo | varies | hosted, your problem | n/a | hosted | token-aware | host-specific |

¹ Optional GitHub token for higher rate limits / private repo search;
optional keyed search providers (`brave`, `google_cse`) that are off
until a key is set. The default surface is keyless.

² "Partial" means: every local computation works offline (chemistry,
biology, nuclear, machinist, math, units, formulas, charts). Search
and retrieval obviously need the upstream they're querying. The
headless browser is always compiled in but Chrome is only needed at
runtime when you actually render — see [building.md](building.md).

## Per-tool / per-host deep dives

### SearXNG

The closest peer for the **search** half of lodestone. SearXNG
aggregates ~200 engines, including many that lodestone doesn't have a
provider for, with strong, mature ranking. It's the right pick when:

- you want the broadest, best-ranked **general web** search,
- you're comfortable running Python + Redis,
- you don't need code/forge search, retrieval helpers, container
  lookups, or local-system tools,
- and you're willing to wrap it for MCP (or use one of the community
  adapters).

Lodestone overlaps but is not the same product. We even support
SearXNG as a *provider* — point lodestone at a self-hosted SearXNG and
it shows up as one of the engines aggregated under `web_search` and
`code_search`. The pitch for picking lodestone instead is: code-aware
out of the box, a single binary, MCP-native, ~400 non-search tools
alongside the search tools, citation-backed science / engineering
surface.

### Brave Search MCP / Tavily MCP / Exa MCP

Hosted-search MCPs. Generally fast, very well-ranked because the
upstream owns the index, and ship as small MCP adapters with the
upstream URL baked in. Pick one of these when:

- you're fine paying for an API key on the default path,
- you want one engine that consistently returns clean, structured
  results without scrape brittleness,
- you don't need lodestone's broader tool surface (system, science,
  memory, etc.).

Lodestone deliberately is not these — the keyless-by-default pitch
falls apart the moment the default surface needs a credit card.
**Brave** and **Google CSE** are available *as keyed providers* inside
lodestone; off until a key is set, never replacing the keyless path.

### Anthropic's `fetch` MCP

Tiny single-purpose server — given a URL, return cleaned text.
Excellent at one thing; nothing else. Lodestone's `fetch_page` /
`render_page` cover the same job (plus `read_pdf`, `fetch_repo_file`
with `owner/repo/path#Lx-Ly` shorthand, archive fallback, etc.) and we
do it inside a server that also gives you the rest of the toolkit.
If you only need URL→text, `fetch` is a one-screen file you can audit
in five minutes — appropriate for that constraint.

### Firecrawl

Hosted crawl + extraction service with structured-output capabilities
(LLM-shaped JSON). When you need:

- production-grade crawling at scale,
- a hosted, billed service,
- LLM-shaped extraction baked into the crawl step,

Firecrawl is the right thing. Lodestone's renderer is **single-page,
on-demand**, intended for "fetch this one stubborn URL through a
browser" rather than "crawl 10k pages on a schedule." We do not
compete with Firecrawl's crawl pipeline; if that's your workload, host
Firecrawl and run lodestone alongside for the rest.

### Official GitHub MCP (`github-mcp-server`)

Token-required GitHub-only server with deep repo / issue / PR /
release surface. The right pick when:

- you spend your time inside GitHub repos, issues, PRs,
- you have a token already and aren't trying to avoid auth,
- you want the official integration vs. a third party.

Lodestone is multi-forge (GitHub + GitLab + Codeberg + Gitea), keyless
on the default path, and focused on the *search + read* slice rather
than `gh`-style operations. We expose `github_releases`,
`github_repo`, `github_user`, `fetch_repo_file`, and `git_run` against
a local working copy — not the full GitHub API surface. If you need
"open this PR", "add this label", "merge this branch": go to the
official MCP.

### Continue / Cline / Aider built-in tools

These IDE / CLI assistants ship their own MCP-style tool surface as
part of the host. The right pick when:

- the only thing you care about is "Claude / Llama can edit my repo,"
- you don't need anything outside of file I/O and shell,
- you're already committed to that host's UX.

Lodestone is the *toolkit*, not the *agent loop*. We don't have a
"plan-and-execute" mode; we expose capabilities and let the host's
model orchestrate. Pair lodestone with Continue / Cline / Claude
Desktop / LM Studio etc. — see
**[docs/setup.md](setup.md)** for per-host config snippets.

### LangChain tool ecosystem

LangChain has hundreds of Python tool implementations. If your app is
already in Python and already runs LangChain, those tools come for
free as Python imports, and you don't need MCP at all. Lodestone is
for the case where the model is running in an MCP host (LM Studio,
Claude Desktop, etc.) that talks to tools over the wire, not inside a
Python process.

## What lodestone is **not**

The shape of the project means it's also deliberately *not* several
things:

1. **Not a paid SaaS.** No accounts, no usage billing, no analytics.
   You self-host.
2. **Not an agent framework.** It doesn't reason, plan, or chain
   tools on its own. The host LLM does. We expose capabilities; the
   model picks. (See [golden rule 2](golden-rules.md).)
3. **Not a large-scale crawler.** Render is single-page, on-demand,
   not a parallel job runner across millions of URLs.
4. **Not a model.** It uses no ML for ranking, summarization, or
   embedding *generation* (the optional embedding feature for semantic
   memory recall points at an *external* endpoint — typically your
   local LM Studio — and is off by default).
5. **Not a stable data source.** Scraping is best-effort; sites
   change. We document the brittleness honestly below.
6. **Not a finished product.** The ~85 skill families and ~400 tools
   represent ongoing work; new families land regularly (see the
   [CHANGELOG](../CHANGELOG.md)). Tools are added when they fit the
   golden rules — see [CONTRIBUTING.md](../CONTRIBUTING.md).
7. **Not a write-tool to third parties** beyond what's documented as
   destructive (`mqtt_publish`, `meshtastic_send`, etc.). No SMTP,
   no Slack, no Discord, no Twitter / X — those write paths fall
   under "wrong project" per the litmus heuristic in CONTRIBUTING.
8. **Not a security tool by design.** The destructive-action guard
   (golden rule 8) and credential redaction (golden rule 11) reduce
   the blast radius of mistakes. They are not an authorization
   system. Run lodestone in a security boundary you understand.

## Honest limitations

This section is deliberately long. Anywhere the project trades
correctness, completeness, or robustness for something else — usually
"works keyless out of the box" or "one binary instead of five" — that
trade is enumerated here. The corresponding source citations or fix
plans live in **[audit-report.md](audit-report.md)** when they're
mathematical, in the per-skill docs under
**[docs/skills/](skills/)** when they're feature-shaped, and in
**[TODO.md](../TODO.md)** when they're roadmap-shaped.

### Search and retrieval

**Scraping is brittle.** The keyless web/code/docs/qa engines render
or parse HTML from the upstream site. Sites change markup; parsers
break. We address this through:

- **Fallback chains** — every kind has multiple providers in priority
  order; the registry walks them.
- **Aggregate strategy** — running all providers in parallel and
  re-ranking ([docs/ranking.md](ranking.md)) gives the system more
  signal even when one provider is sick.
- **Wayback fallback** — `wayback_fetch` lets the model reach an
  archived snapshot of a page that 404s on the live web.
- **Render fallback** — `render=true` per call routes the fetch
  through headless Chrome. Slow but resilient.

But: if every web engine in your stack rate-limits you on the same
day, the model gets nothing useful. The honest answer is "scrape is
the default; rendering is the fallback" — see
[golden rule 1](golden-rules.md).

**DuckDuckGo and Google rate-limit datacenter IPs aggressively.**
Running lodestone in a cloud VM with default settings will, at
unpredictable intervals, hit captcha walls or empty result pages from
DDG. Mitigations:

- run lodestone on a residential connection,
- enable per-provider proxy rotation via `[providers].web` config,
- or set a keyed engine (Brave / Google CSE) and pay for the
  reliability.

**StackExchange keyless API has a daily quota** (~300 requests/IP/day
without a key). Heavy use will trip the quota and the `qa_*` tools
will start returning rate-limit errors. Get a free StackExchange key
and set `[stackexchange].key` to raise the cap.

**`google` engine is browser-only and strictly opt-in.** Google has
no scrapeable endpoint that we can speak to over plain HTTP. Setting
`[providers].google = true` enables it only if Chrome is available; it
always renders. The latency is meaningfully higher than DDG/Mojeek.

**`render_page` adds a Chrome process to the latency.** A single
render call is ~1-3 s on a warm-process renderer, ~5-8 s when Chrome
isn't loaded. The renderer is shared and persistent
([browser.rs](../src/browser.rs)) so the second call is faster, but
parallel renders contend for one Chrome instance.

**`read_pdf` is text-only.** No OCR, no image / table extraction. If
your PDF is scanned, this tool will return nothing useful — pair with
an external OCR step.

**`fetch_page` truncates by character count.** Default 8000,
configurable per call via `max_chars`, with a server-wide cap. Long
documents come back partial; the model has to re-call with `offset`
(not implemented yet) or page through. This is a known UX gap.

**`wayback_fetch` depends on the Internet Archive being up.** When
archive.org is degraded — and it has been, repeatedly, under DDoS —
the fallback path is unavailable. There is no second-order fallback.

### Search ranking

**Composite re-ranking is heuristic, not learned.** The
RRF + consensus + relevance + authority + diversity weighting
([docs/ranking.md](ranking.md)) is hand-tuned to default values that
work decently across a range of queries, but a true learning-to-rank
system would do better on any single query. We do not maintain such a
model.

**No personalized ranking.** Same query produces the same ranking
regardless of which user / model is asking. By design — there is no
user database.

**No query expansion.** "rust async" and "asynchronous programming
in rust" produce different result sets even though a learned system
would group them. Mitigated by `aggregate` strategy across multiple
providers.

### Local-system control

**Filesystem / shell are off by default for safety, but when you turn
them on they are powerful.** A `shell_run` with the default allowlist
empty means "any command." A `fs_write` against `[filesystem].roots`
can overwrite arbitrary files inside those roots. The
**confirmation guard** (golden rule 8) is the load-bearing safety net
— do not disable it by setting `allow_destructive` family-wide
without thought. See the security section of each skill doc.

**Docker / Kubernetes use the daemon socket / kubeconfig** lodestone
runs against. Lodestone has no concept of "which Kubernetes namespace
is the user allowed to touch" beyond what the kubeconfig context
itself permits. If you point lodestone at a privileged kubeconfig, an
LLM can `k8s_delete` namespaces. Scope the context narrowly.

**Database tools (`db_query`, `redis_command`)** accept a connection
URL **per call**. Lodestone does not store these URLs and does not
have a "preconfigured connection list." That is deliberate — it
limits how much credential material lives in the server. But it also
means there is no `allowed_databases` allowlist; whatever URL the
model sends is what gets connected to.

**`systemd_*` is Linux-only.** It's a no-op on macOS and Windows;
the family doesn't even show up on those hosts (the capability
probe gates it).

**`docker_*` requires a reachable Docker daemon** and `kubernetes_*`
requires a parseable kubeconfig. If neither is true, the families
return capability-unavailable errors at call time, not silently fail.

**Package manager calls (`packages` family)** do not auto-install
sudo prerequisites. If your package manager needs `sudo`, the calling
user needs passwordless sudo configured. This is documented in
[`docs/skills/packages.md`](skills/packages.md).

### Math / science / engineering tool families

This section is the longest because the tools themselves carry the
most domain-specific assumptions. Every limitation here is also in
the relevant per-skill doc, and the formulas that produced wrong
answers in earlier releases are tracked in
[`audit-report.md`](audit-report.md).

**Chemistry (`chem_*`):**

- The vendored periodic-table data is IUPAC CIAAW 2021 abridged
  weights. Pinned to that revision. When CIAAW publishes a new
  abridged table, the vendored constants will need refreshing.
- The equation balancer uses **exact integer fraction-free
  Gauss-Jordan**, so the result is exact within the modeled chemistry.
  But it is mass-balance only — **charge balance for redox
  half-reactions is not handled.**
- `chem_ph` uses the small-x approximation `[H⁺] ≈ √(Ka · C₀)` for
  weak acids. Accurate to within a few % for typical undergraduate
  problems; off by more at very low concentration / high pKa.
- `chem_buffer` is Henderson-Hasselbalch; assumes ideal solution
  behavior. Real buffers near their pKa with strong ionic strength
  diverge.
- Temperature defaults to 25 °C (Kw = 10⁻¹⁴). Kw vs T is not
  modeled.

**Biology (`bio_*`):**

- NCBI standard genetic code (table 1) only. Mitochondrial /
  bacterial / vertebrate-mitochondrial codes are not modeled — a
  call to `bio_translate` against mitochondrial DNA will produce a
  protein with stop codons where they shouldn't be.
- ORF finder only emits **complete** ORFs (ATG → in-frame stop).
  Partial ORFs running off the end of the supplied sequence are
  intentionally dropped to avoid ambiguous boundaries.
- `bio_protein_mw` is **monoisotopic** mass using Unimod/Expasy
  residue masses. For average mass the residue table differs — don't
  mix.
- Selenocysteine (U) and pyrrolysine (O) recoding (UGA/UAG) is not
  modeled; they translate as `*`.
- Alignment scoring is linear gap with scalar match/mismatch/gap. For
  production protein alignment with BLOSUM62 + affine gaps, this tool
  is not the right surface — use a dedicated aligner.
- `bio_pcr_tm` offers Wallace (≤ 14 nt) and basic Marmur (15-50 nt).
  No nearest-neighbor SantaLucia model, no salt / Mg corrections, no
  formamide / DMSO terms. This is a first-cut Tm, not Primer3-grade.

**Nuclear physics (`nuke_*`):**

- The SEMF is **Krane's coefficient set** (a_V=15.5, a_S=16.8,
  a_C=0.72, a_A=23.0, a_P=34 MeV; k_P = −3/4). Krane vs Rohlf vs more
  recent fits disagree by ~10 %; the choice is documented in the
  module top-doc. Don't mix coefficient sets across tools.
- The vendored nuclide table is a **curated subset of ~30 species** —
  rare or short-lived isotopes are not present. For those, use NNDC
  NuDat 3 directly and pass masses into `nuke_q_value`.
- `nuke_decay_chain` is the **closed-form Bateman two-step** only.
  Three-step and longer chains aren't modeled; an LLM that needs
  them has to chain pairwise calls and accept the approximation that
  introduces.
- Fission Q-values are exposed as the total / recoverable distinction
  but the antineutrino-energy share varies between references; we
  cite Madland (LANL 2006) but other sources differ by 1-2 MeV.

**Radiation protection (`rad_*`):**

- `rad_shielding_thickness` uses **NIST XCOM mass-attenuation tables
  log-log interpolated** at the 100/200/500/1000/2000 keV anchor
  points. Outside that range it errors. The result is the
  **narrow-beam** thickness — no buildup factor is applied. For
  production shielding design you must multiply by an appropriate
  buildup factor (often 1.5-4× for thick Pb at MeV energies).
- `rad_dose_rate` assumes a **bare, isotropic point source.** Real
  sources have geometry, encapsulation, and self-absorption that this
  tool doesn't model.
- ICRP 103 vs US 10 CFR 20 occupational limits are **not harmonized**
  on the lens-of-eye limit (ICRP 118: 20 mSv/y; NRC: 150 mSv/y) and
  on declared-pregnancy embryo limits. `rad_occupational_limits`
  returns both; pick the regime that governs your work.
- The vendored Γ constants are from Unger & Trubey (ORNL/RSIC-45/R1,
  1982). Newer compilations differ by a few percent.

**Machinist / mech-eng (`mach_*`):**

- Material properties are **typical certified values** per MatWeb /
  ASM. Real material certificates vary; always check the cert for
  the lot you're cutting / loading.
- `mach_cutting_power` uses Sandvik k_c1 / m_c per material group —
  reasonable for first-cut estimates, but real cutting power depends
  on tool geometry, wear, coolant, vibration, and chip morphology
  that the Kienzle model ignores.
- `mach_bolt_torque` uses Shigley Table 8-15 K factors. The "dry as
  received" K = 0.30 is what Shigley publishes; the popular 0.20
  default differs because it's actually the lubricated value. The
  description spells this out.
- Surface-finish formula is **theoretical**. Real Ra is 20-50 %
  higher due to vibration, built-up edge, and runout.
- The thread + tap-drill tables are **75 % engagement, carbon
  steel-targeted.** Soft materials (Al, brass) want larger tap drills
  (≈ 65 % engagement).

**CNC / SCAD (`gcode_*`, `scad_*`):**

- The G-code dialect is the **RS-274/NGC v3 intersection** with Grbl
  and Marlin's known quirks. Output is portable across LinuxCNC and
  Grbl; Marlin needs the simpler tool subset and is documented as
  such. We do **not** emit canned cycles (G81/82/83) because Grbl
  doesn't support them — every drill is emitted as explicit G0/G1
  sequences.
- `gcode_parse_summary` is a syntax-level parser. It does not
  validate against your specific machine's modal-state requirements;
  a program that summary-parses fine can still error on the
  controller.
- OpenSCAD output is plain `.scad` source. We don't render — pipe to
  `openscad -o out.stl model.scad` if you need the mesh.

**Geodesy (`geo_*`):**

- WGS84 is the only datum. For NAD83 / ETRS89 / TWD97 etc., apply a
  `geo_helmert` transform with the relevant parameter set.
- The Bowring closed-form ECEF→geodetic is accurate to ≈ 0.1 mm
  except very close to the poles. For sub-mm precision at the pole
  use Heikkinen 1982 or Vermeille 2002 — out of scope.
- The 0.1.6 audit fixed an MGRS row-letter alternation bug that
  affected ~2/3 of UTM zones. The current code is correct, but
  **MGRS-inverse northing reconstruction near band edges can still
  pick the wrong "ladder"** when the band-center heuristic doesn't
  match the input band. Robust fix is queued.

**Navigation aiding (`nav_*`):**

- Saastamoinen tropospheric delay omits the `B(h)` and `δR(h, z)`
  height-dependent terms. Acceptable to ~5 mm at sea level; off by
  tens of cm at altitude or low elevation. The `height_m` arg is on
  the schema for forward-compat but currently unused — flagged in
  the description.
- Klobuchar ionospheric delay is the single-frequency GPS model. It
  does **not** apply to dual-frequency receivers (which use the
  ionosphere-free combination) or to non-GPS systems.
- IMU drift uses a flat RSS of ARW + bias + scale; Allan-variance
  decomposition (quantization, ARW, bias instability, rate-random-
  walk, ramp) is the correct treatment for any real spec. The flat
  RSS over-estimates short-term and under-estimates long-term error.

**Trajectory mechanics (`traj_*`):**

- The projectile integrator is **RK4 for velocity, semi-implicit
  Euler for position.** Documented; acceptable for `dt ≤ 0.01 s`. Pure
  RK4 for both would require coupling stages.
- Impact detection is "first negative y, post-`t > 0`" — overshoots
  by up to one `dt`. Linear-interpolate to `y = 0` for sub-step
  precision (also queued).
- `traj_hohmann` is the two-impulse, coplanar, ideal-impulse transfer.
  Real transfers have plane changes, gravity losses, finite-burn
  losses. Useful ballpark; not a mission-plan substitute.
- Sutton-Graves stagnation-point heating uses K = 1.74e-4 (the often-
  cited value); Tauber 1989 gives K = 1.7415e-4 for higher precision.
  We use the rounded value.

**RF link engineering (`rf_*`):**

- ITU-R P.676 atmospheric absorption is the **simplified Liebe-style
  heuristic**, not the official text of Annex 2. Match to full P.676
  deviates near the 22 / 60 / 118 / 183 / 325 GHz lines. The
  description disclaims this.
- ITU-R P.838 rain attenuation `k`, `α` are a **single-quadratic
  log-log fit**, not the 4-term lognormal fit in the official
  Recommendation. Reasonable for 1-40 GHz at moderate accuracy;
  meaningfully off above ~80 GHz.
- Hata applies only in its published band (150-1500 MHz, 200/400 MHz
  branches for large cities). The 200-400 MHz gap is undefined in
  the original paper.
- Two-ray asymptotic formula is **frequency-independent** at long
  range. For close-in distances where the breakpoint matters, use
  the full two-ray formula (not implemented here).

**Radar (`radar_*`):**

- Non-coherent integration loss is approximated as `5·log(N)/(N+1)`.
  Reasonable only for small N (~ ≤ 10); too aggressive for large N.
  Use the tabulated Barton/Marcum loss for production.
- OS-CFAR α is a **CA-style approximation in k**, not Rohling's exact
  beta-function solution. Documented as approximate.
- K-distribution clutter threshold is heuristic (tied to Rayleigh at
  ν→∞); no closed-form for the tail. Documented.

**DSP (`signal_*`, `dsp_advanced_*`):**

- BER curves for M-QAM in Rayleigh are an unmotivated `ber_awgn × 4`
  fallback; the closed forms exist (Simon & Alouini Ch. 9). Queued.

**Acoustic (`acoustic_*`):**

- Mackenzie sound-speed formula is validated for T 2-30 °C, S 25-40
  PSU, depth 0-8000 m. Outside that envelope, accuracy degrades.
- Thorp absorption is reasonable for 0.1-1000 kHz in seawater; for
  fresh water or above 100 kHz the model underestimates.

**Information theory (`it_*`, `code_*`):**

- `crc16-ccitt` resolves to the **KERMIT** parameter set
  (poly 0x1021, reflected, init 0x0000). XMODEM and CCITT-FALSE are
  different variants; description spells this out. If your protocol
  needs XMODEM, currently you must reach for `crc16-modbus`-style
  workaround or wait for the alternative variants to land.
- Reed-Solomon is GF(2⁸) only; longer codes need GF(2¹⁶), not
  exposed.

**Crypto math (`crypto_*`):**

- This family is **educational / numeric**, not a TLS / KMS surface.
  Tools are reachable as math, not as a secrets vault. See the
  module top-doc.
- `crypto_jwt_decode` does **not** verify the signature. Description
  flags this loudly.
- PBKDF2 / Argon2 default parameters are **reasonable but pick your
  threat model.** OWASP 2023 floor for PBKDF2-HMAC-SHA-256 is 600 000
  iterations; we recommend that in the description. Setting fewer
  iterations is the caller's decision.

**Constants (`physics.rs`):**

- Six constants (μ₀, ε₀, m_e, m_p, k_e, R_∞) are CODATA 2018, not
  CODATA 2022. All discrepancies are at <1e-8 relative — functionally
  negligible for engineering, off in the last few digits for
  metrology. Queued.

### Memory and recall

**`memory_*` and `solution_*` are best-effort, advisory.** Recall is
fuzzy (token + synonym + optional semantic), and the model is told to
treat hits as suggestions, not authoritative answers. False positives
happen — the recall preamble explicitly says "verify before reusing."

**Embedding-based recall is opt-in and external.** When
`embedding_endpoint` is set, lodestone POSTs query text to that
endpoint to get vectors. The endpoint is typically your local LM
Studio; lodestone does not embed text itself.

**Conversation logs accumulate on disk** under `[memory].dir`.
Default `.lodestone-memory/`. Retention is unbounded unless you set
`conversation_retention_days` or `max_conversations`. Auto-prune at
startup is opt-in. If you delete the directory, history is gone.

### Constellation (P2P share)

**The constellation is opt-in** and off by default. When on, it
shares **only hashes** of normalized query keys over the wire — never
raw queries — and only **cached search results / fetched bytes** from
keyless sources, never keyed-provider payloads.

**Bloom filters have false positives.** A peer's digest may suggest
it has a key when it doesn't; that consultation will then return
nothing and we fall through to the local fetch. Pure overhead, never
incorrect.

**Reputation is in-memory.** A peer's score does not survive a
restart; v1 makes that choice explicit. After restart you trust
default peer scores for `sync_secs` until reputation rebuilds.

**Consensus gate (`min_agreement`) caps single-peer influence.** A
result is served by the mesh only when `min_agreement` peers
corroborate it. A malicious peer cannot solo-poison the cache; it
can at worst suppress consensus by abstaining.

**mDNS discovery is best-effort.** Subnets, switches, and wireless
APs vary in how reliably they pass mDNS. If your LAN drops mDNS, the
static `[network].peers` list is the resilient path. Documented in
[constellation.md](constellation.md).

**Constellation traffic is HTTP, not HTTPS.** Bearer-token (the
`[network].token`) is constant-time-compared (`ct_eq`) but the wire
is plain. Run on a trust-boundary network or layer your own TLS
terminator in front. mTLS is queued.

### Auth

**Bearer token is the only `/mcp` auth.** Set `auth_token` and every
request must carry `Authorization: Bearer <token>` (constant-time
compared). There is no OAuth, no JWT verify, no client-cert auth on
`/mcp`. For finer-grained access you wrap lodestone behind a proxy.

**Tokens are global.** "User A can call `db_query` but not
`shell_run`" is not modeled — if the bearer is valid, all enabled
tools are reachable. Per-tool ACLs are a roadmap item, not in 0.1.6.

### Performance / scale

**Single-threaded async runtime backed by Tokio.** Concurrent
requests are fine; the math tools are pure-CPU and won't starve the
runtime because they're fast, but a 100 ms sync FFT on the async
thread isn't great practice. Heavy computation should be moved to a
blocking pool — not all tools do this yet.

**No GPU acceleration.** All FFTs, alignments, linalg are CPU. For
production at scale, point to a different toolchain.

**No streaming for large results.** A `read_pdf` against a 200 MB
PDF processes the whole document, hits the `max_chars` truncation,
and returns. There is no "give me the next page" API.

**No caching of math results.** Pure-math tools are cheap enough
that we don't cache; the same call twice executes twice. The
constellation cache is for **retrieval** outputs, not math.

**Memory footprint on the lookup tables.** Periodic table, AME2020
nuclide subset, NuDat 3 isotope table, MatWeb material table, ASTM
E140 hardness table, thread table — all vendored in the binary. Adds
~50 KB to the binary; trivial. Larger tables (full WMM grids, full
NRLMSISE-00 coefficients) are deferred for that reason.

### Documentation drift

**Doc-content lag.** The README / docs are updated in the same
commit that lands a feature; the `audit-report.md` is updated when a
formula changes; the CHANGELOG ledger is canonical. If a doc claim
disagrees with the code, the code is the source of truth. File an
issue.

**Per-skill docs are per-skill.** Each family has its own page under
[`docs/skills/`](skills/), with citations and example uses. Reading
all 85 pages is impractical; the indexes
([skills.md](skills.md), [tools.md](tools.md)) are the entry points.

**No videos / no screencasts.** This project is text-first.

### LLM-side limitations

These are **not** lodestone limitations — they're host/model
limitations that affect what you can do with lodestone:

- **Tool-selection accuracy varies wildly by model.** A small local
  model often picks the wrong tool, hallucinates arguments, or
  ignores tools entirely. Lodestone exposes ~400 tools; this is a
  lot of context. Use a model that's been trained on tool use, and
  set the host's tool-restriction config to limit what shows up.
- **MCP `tools/list` payload is large.** ~400 tools × ~200 bytes
  each ≈ 80 KB per session. Some hosts truncate. Disable families
  you don't need via `[tools].disabled` to shrink the surface.
- **Stdio-only hosts need `mcp-remote`.** Documented in
  [docs/setup.md](setup.md).

## When to pick something else

A concise version of the above:

- **SearXNG (+ MCP adapter)** — broadest, best-ranked general web
  search; you don't need the rest of the toolkit; you're fine with
  Python + Redis.
- **Brave / Tavily / Exa MCP** — keyed, hosted, high-quality search;
  you're fine paying for the API key on the default path.
- **Firecrawl** — production-grade crawling at scale, hosted, with
  LLM-shaped extraction baked in.
- **Official GitHub MCP** — token-required deep GitHub surface (PRs,
  issues, releases, gh-style ops). Lodestone is multi-forge and
  keyless-first, focused on search + read.
- **Anthropic `fetch` MCP** — URL → text, nothing else; tiny, easy
  to audit.
- **Continue / Cline / Aider built-ins** — you want a one-host repo-
  editing assistant and don't need anything outside file I/O / shell.
- **LangChain ecosystem** — your app is already Python and already
  runs LangChain.

For everything else — broad toolkit, keyless default, MCP-native,
single binary, citation-backed science / engineering, opt-in P2P
mesh, working out of the box — that is the niche lodestone targets.
