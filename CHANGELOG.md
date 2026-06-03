# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project aims to
follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.11] - 2026-06-02

Three new skill families landing 11 tools targeting tasks other LLMs
measurably fail at: CIDR / subnet math, cron expression
description/iteration/validation, and UUID generation/parsing/short-form
encoding. All three are pure-compute, keyless, on by default, and
`retrieval_policy::None`. No new credentials, no new external services.

### Added — `network` family (5 tools)

Binary subnet math that LLMs get wrong roughly half the time on
intermediate cases (off-aligned CIDRs, mixed v4/v6, IANA special-purpose
categorization).

- **`net_cidr_info { cidr }`** — network address, broadcast (v4),
  netmask, wildcard mask, host range, total addresses, usable hosts,
  prefix length. Off-aligned hosts (e.g. `10.42.7.83/19`) truncate to
  the canonical network in the output.
- **`net_cidr_subnets { cidr, new_prefix, limit? }`** — split a CIDR
  into equal-sized subnets at a longer prefix. `limit` (default 256)
  bounds the output so a /16 → /32 split doesn't dump 65 536 rows.
- **`net_ip_in_cidr { ip, cidr }`** — membership test + the 0-indexed
  position of the IP within the block when contained.
- **`net_ip_classify { ip }`** — public / private (RFC 1918) /
  loopback / link-local / multicast / CGNAT (RFC 6598) / documentation
  (RFC 5737, RFC 3849) / unspecified / reserved (v4) / unique-local /
  TEREDO / 6to4 (v6). Each category cites the governing RFC.
- **`net_cidr_summarize { cidrs }`** — coalesce a list of CIDRs into
  the minimal covering set (v4 and v6 aggregated separately).

Backed by [`ipnet`](https://crates.io/crates/ipnet); no extra deps
beyond what was already in the tree. `FamilyMeta::example_flow()`
walks classify → info → subnets → summarize. Docs at
[`docs/skills/network.md`](docs/skills/network.md).

### Added — `cron` family (3 tools)

The DOM/DOW interaction in Vixie cron is a recurring LLM stumble; this
family spells it out explicitly.

- **`cron_describe { expression, timezone? }`** — plain-English
  description plus the next 3 firings when the expression iterates.
  When both DOM and DOW are restricted, the output names the Vixie
  OR semantics ("fires when EITHER constraint matches") — that's the
  rule LLMs get backwards. Accepts 5-field (`min hour dom month dow`)
  and 6-field (`sec min hour dom month dow`) form; 5-field gets
  `0 ` prepended for seconds before the underlying parser.
- **`cron_next { expression, count?, from?, timezone? }`** — next N
  firings as ISO timestamps. `count` defaults to 5 (max 100); `from`
  defaults to now.
- **`cron_validate { expression }`** — parse + return `valid=true` or
  a precise error pointing at the bad field.

Backed by [`cron`](https://crates.io/crates/cron) (Quartz-style
iterator) + a small custom describer. The underlying iterator won't
evaluate expressions that restrict both DOM and DOW (Quartz requires
one to be `?`); `cron_describe` handles that gracefully — the English
description still renders and the response carries an
`iteration_note` explaining why no firings are listed. Docs at
[`docs/skills/cron.md`](docs/skills/cron.md).

### Added — `uuid` family (3 tools)

LLMs hallucinate UUIDv7 field layouts more often than not. This family
gives them deterministic generators and a parser that reads the
actual RFC 9562 bit layout.

- **`uuid_generate { version, count?, at? }`** — generate v4 (random,
  RFC 9562 §5.4) or v7 (Unix-millisecond timestamp + entropy, RFC 9562
  §5.7). `count` defaults to 1, max 1000. `at` (v7 only) embeds a
  specific RFC3339 moment instead of "now"; truncated to ms precision
  per spec.
- **`uuid_parse { uuid }`** — version (1-8) + variant (RFC 4122/9562,
  Microsoft, NCS, future) + canonical forms (hyphenated, hex-only,
  urn:uuid:). For time-based versions (1, 6, 7) the embedded timestamp
  is decoded and returned as RFC3339 + Unix ms.
- **`uuid_to_short { uuid, encoding }`** — re-encode into `base32`
  (26 chars), `base58` (Bitcoin alphabet, ~22 chars, no visually-
  confusing characters), or `base64url` (22 chars, URL-safe alphabet,
  no padding).

Backed by [`uuid`](https://crates.io/crates/uuid) with `v4` and `v7`
features. Docs at [`docs/skills/uuid.md`](docs/skills/uuid.md).

### Tests

20 new unit tests across the three families (`network`: 8,
`cron_expr`: 6, `uuid_tools`: 6). Total `cargo test` count 375
(355 → 375), clippy `--all-targets -D warnings` clean, fmt clean.

## [0.1.10] - 2026-06-02

### Changed — handshake footer collapses redundant lookup-tools list

The bottom `WHERE TO LOOK UP DETAILS` section was repeating the top
`LOOKUP TOOLS` section almost verbatim — same four tools
(`describe_skill`, `describe_family`, `features`, `list_providers`),
slightly different phrasings, ~340 chars of pure redundancy. The
per-family inventory between them already constantly references
`describe_family name="..."`, so the lookup pattern stays fresh
through the body of the handshake; a second exhaustive copy at the
bottom paid for nothing. Replaced with one line that points back at
the top section. Handshake size 14,178 → 13,837 chars.

## [0.1.9] - 2026-06-02

Full-catalog fill of `examples()` and `use_cases()` on every Skill impl,
the dynamic instructions handshake gains an introductory preamble and a
prominent lookup-tools section, and a clutch of accuracy bugs the
fill-time verifier caught are now fixed.

### Added — `examples()` and `use_cases()` on 432 of 466 Skill impls

A multi-agent workflow (11 fill agents in parallel, each followed by a
quality-gate verifier auditing against `algebra.rs` as the reference)
walked the entire skill catalog and added 2–4 worked-example
invocations plus 2–4 use-case phrases to every `impl Skill for X`
block. Coverage is now **432 / 466** (93 %); the residual 34 are
ambiguous edges where field semantics weren't legible enough to write
a plausible example. Each example carries valid JSON args matching
the Args struct's actual field names; each use-case phrase says when
to reach for *this* tool over a sibling.

The fill is consumed by the existing `describe_skill` tool (one
tool's full schema + examples on demand) and folded into the
dynamic instructions handshake. The MCP `tools/list` payload is
unchanged — coverage costs nothing until an LLM asks.

### Changed — handshake leads with a preamble and a LOOKUP TOOLS section

`get_info()`'s dynamic handshake now opens with a proper welcoming
paragraph ("Welcome to Lodestone MCP server vX.Y.Z. This is a
keyless, self-hosted toolkit…") followed by a live `{active}/{total}`
count and the ~100-families organization concept. A new **LOOKUP
TOOLS** section sits right after the preamble, before any other
guidance — it explicitly calls out `describe_skill`,
`describe_family`, `features`, and `list_providers` by name with a
one-line summary of when to use each. The previous `GENERAL
APPROACH` and per-family inventory sections follow.

### Fixed — accuracy bugs caught by the verifier

The quality-gate verifier flagged 25 issues across the fill output
(4 high, 12 medium, 9 low). All highs and the load-bearing mediums
are addressed here:

- **`chart_canvas`** — `CanvasCommand` is `#[serde(tag = "kind")]`
  but had no `rename_all`, so serde expected PascalCase variant
  names (`Rect`, `Line`, …) while every example used lowercase
  (`rect`, `line`, …). Added `#[serde(rename_all = "lowercase")]` so
  the lowercase tokens the LLM naturally writes round-trip cleanly.
- **`sat_position` / `sat_observe` / `sat_passes`** — examples used
  a 2024-epoch TLE with the 2008 checksum digit, so
  `sgp4::Elements::from_tle` rejected them with `Bad line checksum`.
  Switched to the canonical SGP4 verification-suite TLE (2008
  epoch, correct checksum) and moved every `at` / `from` time to be
  near that epoch so SGP4's ~1–2 week validity window holds; notes
  now point at Celestrak / Space-Track for live use.
- **`tasks_get` / `tasks_result` / `tasks_cancel`** — placeholder
  task IDs `tsk_01HXYZ...` would never match the runtime's actual
  `task-{n}` format. Updated to `task-3`.
- **`docker_run`** — example had `"command": "sh -c 'echo hi'"`
  with a misleading "no host shell" note; the implementation
  splits on whitespace, so the embedded quotes would arrive
  literally. Switched to a plain command and rewrote the note to
  spell out the whitespace-split + no-shell behavior.
- **`disasm_x86_hex`** — the field doc-comment promised it strips
  `0x` prefixes but `parse_hex` only filtered non-hex-digit
  characters (so a literal `0x90` became `090`, an odd length, and
  errored). Fixed `parse_hex` to split on whitespace/comma and
  strip `0x`/`0X` per token. Description now matches behavior.
- **`astro_sun`** — solstice-noon example was at lat=0 with a note
  claiming altitude ≈ 90°, but the Sun's declination is +23.44° at
  June solstice, so equator noon altitude is ~66.6°. Moved the
  example to the Tropic of Cancer (lat=23.43°) where the note is
  actually true.
- **`astro_visible_stars`** — note said "brightest first" but the
  implementation sorts by altitude (highest first). Corrected.
- **`physical_constant`** — example `{"name": "c"}` was advertised
  as returning the speed of light entry, but `name` is a
  case-insensitive substring filter against both symbol and name,
  so the letter `c` matches every constant containing `c`. Switched
  to a discriminating filter (`speed of light`) and rewrote the
  note to explain substring matching.
- **`acoustic_snell`** — air-to-water example note said "beyond the
  critical angle" without naming it; rewritten to spell out the
  ~13.2° critical angle so 30° is recognizably past it.
- **`eia_series`** — example labelled "Henry Hub natural gas spot
  price" actually used the NYMEX futures path (`pri/fut`).
  Re-titled to match the actual dataset and pointed at the spot
  path (`pri/sum`) in the note.
- **`interchange_stl_info`** — placeholder `data_base64` (`AAAAAA...==`)
  was neither valid base64 nor long enough for the 84-byte STL
  header, so the example would never run. Replaced with a real
  single-triangle ASCII STL and called out the base64 path in the
  note instead.

Filesystem field doc-comment for `bytes_hex` now matches the
parser's promise (the change to `parse_hex` makes the description
literally true rather than aspirational).

### Resolved verifier-skipped clusters

Three clusters (`search-retrieval`, `rf-signal-radar`,
`bio-chem-trajectory`) had their fill-stage verifier skipped because
of a transient parallel-edit conflict in a different cluster's
chart.rs intermediate state. A follow-up read-only audit of all
three (88 skills total) found 0 high-severity, 1 medium, 6 low
issues. The medium (the acoustic_snell critical-angle note) is
fixed above; the lows are stylistic nits left for the next pass.

## [0.1.8] - 2026-06-02

LLM-facing surface gets two new contracts (`Skill::examples`,
`Skill::use_cases`, and `FamilyMeta::example_flow`), two new
introspection tools (`describe_skill`, `describe_family`), and a
dynamic instructions handshake that replaces the hand-curated
`docs/instructions.md`. Plus a generous-parser pass on
`algebra_solve` / `arithmetic_eval` to absorb the LLM-typical shapes
those tools were rejecting.

### Added — `Skill` trait gains `examples()` and `use_cases()`

`Skill::examples()` returns `&'static [SkillExample]`; each entry is
a `{title, args: JSON literal, note: Option<&str>}`. `use_cases()`
returns `&'static [&'static str]` — short phrases naming the
situations the tool is the right answer for. Both default to `&[]`,
so all 470-ish skills compile without change. They are **not** part
of `tools/list`; they surface only through `describe_skill` and the
session-start instructions handshake, so the payload cost is paid
only when an LLM asks for it.

Curated exemplars landed on 10 representative skills as the
reference pattern: `algebra_solve`, `fs_write`, `docker_pull`,
`arxiv_get`, `web_search`, `recall`, `memory_save`, `atm_isa`,
`nasa_neo`, `osm_overpass`. The contract is documented in
[CONTRIBUTING.md](CONTRIBUTING.md#3-document-it); fills will land
incrementally.

### Added — `FamilyMeta::example_flow()`

Multi-tool worked example per family — a numbered list of 2–5
chained calls representing the canonical task. Defaults to `None`.
Filled today on `docker::Family` (bring up a service),
`kubernetes::Family` (scale a deployment), `filesystem::Family`
(triage and edit a file — Family newly registered), and
`packages::Family` (manager → search → info → install). Surfaced
through `describe_family` and the dynamic handshake.

### Added — `describe_skill` and `describe_family` meta tools

- **`describe_skill { name }`** returns one tool's description, use
  cases, examples, family context, family gating state, and the
  full per-property JSON Schema as pretty JSON. Useful when the
  host truncated the initial `tools/list` payload (~80 KB at this
  tool count) or when the LLM wants to double-check a tool's
  argument shape without re-reading the whole catalog.
- **`describe_family { name }`** returns the family's description,
  capability state, gating state, full tool list, and the
  registered example flow. Falls back to grouping by tool-name
  prefix when a family hasn't registered `FamilyMeta`.

Both live in `src/skills/meta.rs` alongside `features`,
`list_providers`, and the `constellation_*` introspection tools.

### Changed — dynamic instructions handshake

`get_info()`'s static `include_str!("../docs/instructions.md")` is
gone, replaced by a `build_instructions(&Config)` function that
reflects the live registry at session-start:

1. A prologue covering the general approach + the new introspection
   tools.
2. A per-family inventory walking every `FamilyMeta` registration —
   description, host-capability state, active-tool count, and a
   pointer at `describe_family` (with a flag noting when an example
   flow is registered).
3. "Other tool families" — skills without `FamilyMeta`, grouped by
   leading `<prefix>_` segment with a sample of tool names.
4. A footer pointing at `features`, `describe_skill`,
   `describe_family`, and `list_providers`.

The hand-curated `docs/instructions.md` is no longer included by
the build but is preserved in the tree as a long-form reference for
new contributors.

### Added — `arithmetic_eval` + `algebra_solve` accept LLM-typical input

Observed inputs the tools were rejecting before:

- `arithmetic_eval: 2**10`
- `algebra_solve: x^2 / 9.81 = 5000 -> solve for x (velocity needed for 5,000 km range)`
- `algebra_solve: x^2 / 9.81 = 5000000, find x`
- `algebra_solve: s = u*t + 0.5*a*t^2 where s=1000, u=800, a=-9.81 -> solve for t`
- `algebra_solve: 800*t - 4.905*t^2 = 1000`

`arithmetic::normalize` now rewrites Python-style `**` to meval's
`^` before parsing.

`algebra_solve` now:
- Strips trailing prose (`, find X`, `-> solve for X`, and any
  parenthetical description following).
- Parses a `where var=val, var=val, ...` substitution clause and
  substitutes named parameters before solving.
- Auto-detects the free variable (numbers stripped first so `2e3`
  doesn't get mis-read as `e3`), so equations in `t`, `v`, `n`,
  etc. work alongside `x`. Multiple free vars produce a clear error
  pointing at `where`; zero free vars points at `arithmetic_eval`.
- Inserts implicit `*` around the detected variable (`2t` → `2*t`),
  not just `x`.

### Added — `Args` struct field doc-comment audit

Swept the 405 `Args` structs under `src/skills/` and filled in
missing `///` doc-comments on individual fields — 227 fields across
30 files. Combined with the `schemars::JsonSchema` derive, every
per-property `description` in `tools/list` is now uniformly
populated. Only `GlobalToolArgs.background` in `src/skills/mod.rs`
remains documented out-of-band (its schema fragment is hand-built
in `merge_global_args_into_schema()` with an explicit description).

## [0.1.7] - 2026-06-02

Cross-codebase **golden-rules audit** of every skill, fanning out four
parallel research agents over the search/retrieval, local-system,
utility/data-feed, and science/math clusters. Findings led to a tight
sweep of high-severity guard gaps and one per-user cache carve-out, plus
a three-instance smoke harness that exercises every constellation
surface and every confirmation-guard branch end-to-end. Tool count is
now ~470 across ~100 skill families.

### Fixed — destructive operations that ran without a confirmation guard

Seven tools advertised as destructive but slipped past `guard::check`
entirely; the audit flagged each as **high-severity**, since the cost of
a single un-prompted invocation ranges from arbitrary container exec to
silent file overwrite. All eight now route through `guard::check` with
the family's `allow_destructive` flag as the pre-authorized override
(same pattern as `docker_stop` / `k8s_delete` / `fs_delete`):

- **`docker_pull`** — silent network egress to an arbitrary registry +
  writes to the local image store.
- **`docker_run`** — creates and starts a container; effectively
  arbitrary code execution under the image's entrypoint.
- **`docker_start`** — resumes a stopped container that may bind ports,
  mount volumes, or execute its entrypoint.
- **`docker_build`** — every Dockerfile `RUN` step is arbitrary code
  execution under the daemon.
- **`k8s_apply`** — server-side apply of arbitrary cluster state (RBAC,
  workloads, secrets).
- **`k8s_scale`** — replica patches can take a production workload to
  zero silently.
- **`fs_write` (overwrite only)** + **`fs_edit`** — silent overwrite /
  mutation of an existing file. Creating a new file with `fs_write`
  remains innocuous and does not challenge.

### Changed — guard bindings are now per-target on every new call site

Earlier guarded tools key their `trust` on the **tool name** (e.g.
`docker_stop`), so one `trust=true` whitelists every container for the
rest of the session. The eight new guard call sites use **per-target
binding keys** instead, so trust is narrow and discoverable:

| Tool          | Binding key                                           |
| ------------- | ----------------------------------------------------- |
| `fs_write`    | `fs_write:<canonical path>`                            |
| `fs_edit`     | `fs_edit:<canonical path>`                             |
| `docker_pull` | `docker_pull:<image>`                                  |
| `docker_run`  | `docker_run:<image>|<name>|<command>`                  |
| `docker_start`| `docker_start:<container>`                             |
| `docker_build`| `docker_build:<context>|<dockerfile>|<tag>`            |
| `k8s_apply`   | `k8s_apply:<sha-of-manifest>`                          |
| `k8s_scale`   | `k8s_scale:<kind>:<name>:<namespace>`                  |

Tightening the existing tool-keyed bindings (`mqtt_publish`,
`meshtastic_send`, `package_install`, `ffmpeg_convert`, `sheet_write`,
`serial_send`, `store_purge`) is tracked separately and not in this
release.

### Changed — `translate` / `detect_language` walked back from
`RetrievalPolicy::Shared` to `None`

Both tools previously cached translation bodies and advertised them on
the constellation digest. The audit flagged this as a GR-13 carve-out
violation: translation input is arbitrary per-user text that can carry
PII or secret-shaped strings the caller pasted in, and broadcasting the
translated body to constellation peers would leak it. Memo and
conversation history still cover the local repeat-call case.

### Added — `RetrievalPolicy` interface, declared on 48 retrieving skill impls

A common contract for "should downloads through this skill enter the
constellation cache?" lives on every `Skill` impl as the
`retrieval_policy()` method:

- `None` (default) — purely-local compute, never caches.
- `LocalOnly` — caches in-process, never advertises to peers.
- `Shared { source }` — caches in-process and advertises the keyed
  entry on the next constellation digest. The `source` field picks the
  per-source TTL and `min_agreement_floor` (Wayback / Arxiv / Github /
  Overpass / SearchEngine / Other).

48 retrieving skill impls across 24 modules now declare their policy
explicitly. The interface itself is wired but not yet load-bearing —
existing skills keep their hand-rolled `retrieval_get` / `retrieval_put`
flow until a lifetime-clean migration path is in place. See the
`RetrievalPolicy` doc-comment in [`src/skills/mod.rs`](src/skills/mod.rs)
and [`CONTRIBUTING.md`](CONTRIBUTING.md#retrievalpolicy) for the per-skill
guidance.

### Added — golden rules 13 and 14

- **GR-13: helping the greater good.** All remotely-retrieved academic
  / scientific / standards content (arxiv, openalex, unpaywall, pubmed,
  NIST, RFC, OSM/Overpass, NOAA, Open-Meteo, etc.) is shared over the
  constellation by default, so a peer can serve the same record without
  the upstream being touched again. Per-user content (translation,
  memory, conversation history, keyed-source bodies) is explicitly
  carved out.
- **GR-14: diagrams are Mermaid.** Architectural and request-flow
  diagrams in docs are authored as Mermaid (`sequenceDiagram`,
  `stateDiagram-v2`, etc.) so they render in-place on GitHub and stay
  searchable.

### Added — request-flow articulation for the constellation

The constellation now ships seven explicitly-documented request flow
paths (A through G) in [`docs/constellation.md`](docs/constellation.md),
each as a Mermaid `sequenceDiagram`: local hit, local miss + upstream,
peer cache hit, peer cache hit with delegation, retrieval delegation
under rate-limit, blob serving, browser-persona delegation. A separate
end-to-end smoke harness at `smoke/smoke.py` (gitignored) spins up
three lodestone instances and exercises all of them — `/constellation/
digest` auth, `/query` roundtrip, `/blobinfo`, capability-gated
`/retrieve` refusal, MCP `constellation_status` /
`constellation_capabilities` / `constellation_peers`, two-node digest
gossip, and every branch of the confirmation-guard flow (new file,
overwrite challenge, confirm, trust within a path, no leakage across
paths, single-use token, replay rejection). 25/25 pass.

### Defaults — all skill families now enabled by default

Six families that previously shipped `enabled = false` (Python, MQTT,
Meshtastic, Shell, Serial, Store) are now on by default, matching the
posture of the rest of the surface. Off-switches and per-family
`allow_destructive` flags are unchanged.

### Cleanup — `allow(dead_code)` markers carry rationale

Eight `#[allow(dead_code)]` annotations in `src/skills/` that had no
explicit reason comment now carry a one-line justification so the next
clippy sweep doesn't silently delete code that is load-bearing for a
specific future or for the `RetrievalPolicy` interface.

## [0.1.6] - 2026-06-02

Cross-codebase **correctness audit** of every skill that makes factual or
mathematical claims. Six parallel research agents read the relevant Rust
source and cross-checked every formula and vendored constant against
canonical sources (IEEE/ISO/NIST/CODATA, vendor handbooks, original
papers). Findings consolidated in
**[docs/audit-report.md](docs/audit-report.md)**; this changelog covers
the fixes.

### Fixed — wrong-answer bugs

- **`astro.rs` azimuth was south-referenced** (Meeus convention) but
  passed straight to `compass()` (north-referenced). Every astro tool —
  `astro_sun`, `astro_moon`, `astro_star`, `astro_visible_stars` —
  reported its compass direction rotated by 180° (a body due south
  showed up as `N`). **Added 180° to the Meeus azimuth before
  normalizing.**

- **`astro.rs` lunar β third-largest coefficient** was `0.278·sin(M′−F)`;
  Meeus Table 47.B specifies **`0.173·sin(M′−F)`**. ~0.1° declination
  error.

- **`astro.rs` Moon rise/set threshold** was `0.567°` (refraction only).
  The standard altitude h₀ = **+0.125°** accounts for parallax and
  semi-diameter as well (Meeus §15). Earlier value placed rise/set
  several minutes early/late.

- **`satellite.rs` `sat_passes` set-time bisection** passed `(t, t −
  step)` as `(lo, hi)`, inverting the bisector's `lo < hi` contract.
  The bisection walked backward and could return the wrong endpoint by
  up to one step. **Fixed** to pass earlier-then-later, matching the
  rise-time call.

- **`dsp_advanced.rs` M-QAM BER** was missing the `1/√2` factor in the
  erfc argument: `√(3·k·γ/(M−1))` instead of `√(3·k·γ/(2(M−1)))` — see
  Proakis 5e §5.2.9. Understated BER by ≈3 dB effective SNR.

- **`rf_link.rs` Hata large-city threshold** was `f ≥ 200 MHz` (which
  applies the 400-MHz branch throughout the 200–400 MHz band that Hata
  doesn't actually cover). **Fixed to `f ≥ 400 MHz`.**

- **`rf_link.rs` knife-edge docstring** claimed `h positive = clear`
  but the formula `v = h·√(…)` follows the standard "h positive =
  obstructing" convention (Rappaport §4.7; ITU-R P.526). Rewrote
  description and field doc to match the implementation.

- **`geodesy.rs` MGRS row-letter set selection** used `(set / 3) % 2`,
  alternating the row pattern every **3** zones. NGA TM 8358.1 §3.2.2.3
  alternates **every** zone (odd vs even). MGRS strings were wrong for
  ~2/3 of UTM zones in both `mgrs_forward` and `mgrs_inverse`. **Fixed**
  with `(zone − 1) % 2`.

### Changed — guard rails / honesty

- **`linalg_eigen` now rejects non-symmetric input** rather than silently
  averaging `A` and `Aᵀ`. Threshold: `‖A − Aᵀ‖ > 1e-9·‖A‖`. Silent
  symmetrization was hiding caller bugs.

- **`opt_tsp_2opt` now rejects asymmetric distance matrices** with a
  clear error pointing at the violating cell. The 2-opt delta is only
  valid for symmetric distances.

- **`crypto_pbkdf2` description** now cites the current OWASP / NIST
  guidance of **600 000** iterations (was "≥ 100 000 today").

- **`crypto_argon2` description** corrected: defaults are between
  OWASP's floor (t=2, m=19 MiB, p=1) and high-end (t=3, m=64 MiB, p=4)
  — not "OWASP 2023 low-end".

- **`code_crc` description** now spells out which of the three
  "CRC-16-CCITT" parameter sets `crc16-ccitt` resolves to (KERMIT
  variant), and the polynomial behind each algorithm key.

- **`trigonometry` `arc_length` and `sector_area` summaries** rewritten
  to show `s = r·θ·π/180` (the code is correct — it converts
  internally — but the summaries had `s = r·θ (degrees)` which is
  algebraically wrong without the conversion).

- **`convert_units` description** now spells out **US customary** vs
  imperial for volume and **decimal vs binary** prefixes (IEC 80000-13)
  for data.

- **`nav_saastamoinen` description** documents the omitted height-
  dependent `B(h)` and `δR(h, z)` terms (the `height_m` arg stays for
  forward-compat).

- **`geo_latlon_from_ecef` description** rewritten to call out Bowring's
  closed-form single-pass (was "converges in 2–3 iterations") and the
  polar-singularity caveat.

### Acknowledged caveats (no code change)

A handful of findings are documented in
[`docs/audit-report.md`](docs/audit-report.md) Tier 3 — these are
scoped trade-offs (rough but documented approximations) rather than
bugs. Includes: CODATA 2018 vs 2022 constants in `physics.rs`,
haversine antipodal stability, OS-CFAR Rohling approximation,
Holt-Winters seasonal-seed averaging, KF Joseph form, MGRS-inverse
band-edge handling, and the ITU-R P.676/P.838 simplified line fits.

### Test status

- 347 / 347 unit tests pass.
- 163 / 163 tools exercised end-to-end through `Skill::call` in the
  smoketest.
- `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` clean.

## [0.1.5] - 2026-06-02

Nuclear physics, radiation protection / dosage, machinist + mechanical
engineering, and CNC / OpenSCAD generation. Every formula and constant
in the new families was validated against a primary or canonical source
**before** implementation via parallel research agents, and the sources
are baked into both the module-level docs and each tool's
`description()`. Constellation-share invariant established for **all**
scientific data retrieval, retroactive to the bio_data / open_data /
SWPC families landed earlier.

### Added

- **Nuclear physics family** (`nuke_*`, 6 tools): `nuke_nuclide_lookup`
  (vendored AME2020 atomic masses + NUBASE2020 half-lives and decay
  modes for ~30 stable + radioactive nuclides), `nuke_binding_energy`
  (Bethe-Weizsäcker semi-empirical mass formula with Krane defaults:
  a_V = 15.5, a_S = 16.8, a_C = 0.72, a_A = 23.0, a_P = 34 MeV;
  k_P = −3/4), `nuke_q_value` (atomic-mass reaction Q with CODATA 2022
  factor 1 u = 931.494 103 72 MeV/c²), `nuke_decay_law`,
  `nuke_decay_chain` (closed-form Bateman two-step **with the
  L'Hôpital special case for λ_A = λ_B** so the numerics don't blow up
  near equal rates), and `nuke_unit_convert` (u ↔ MeV/c², Bq ↔ Ci,
  barn ↔ cm²).

- **Radiation protection family** (`rad_*`, 10 tools), explicitly
  **general radioactivity** (industrial / research / calibration /
  medical alike, not specifically medical imaging):
  `rad_isotope_lookup` (NNDC NuDat 3 isotope table including
  industrial sources Co-60, Cs-137, Ir-192, Am-241 and calibration
  isotopes Ba-133, Eu-152, Na-22 alongside clinical tracers),
  `rad_units` (Gy ↔ rad, Sv ↔ rem, R ↔ Gy_air via NIST W/e = 33.97 J/C),
  `rad_attenuation` (Beer-Lambert + HVL + TVL), `rad_inverse_square`,
  `rad_dose_rate` (idealized point source from vendored Γ per
  ORNL/RSIC-45/R1), `rad_equivalent_dose` (full ICRP 103 piecewise
  continuous neutron-w_R function), `rad_effective_half_life`,
  `rad_occupational_limits` (ICRP 103 alongside US 10 CFR 20 with
  the unharmonized lens-of-eye limit difference called out),
  `rad_shielding_thickness` (NIST XCOM log-log-interpolated
  attenuation tables for Pb, ordinary concrete, iron/steel, water,
  aluminum at 100-2000 keV anchors), and `rad_alara` (combined
  time × distance × shielding triad).

- **Machinist / mech-eng family** (`mach_*`, 12 tools):
  `mach_cutting_speed` and `mach_feed_rate` (Machinery's Handbook
  31 e.), `mach_mrr_milling`, `mach_cutting_power` (Sandvik Kienzle
  k_c = k_c1 · h_m^(−m_c) with vendored coefficients for Al 6061,
  steel 1020, SS 304, gray cast iron, Ti-6Al-4V),
  `mach_surface_finish_turning` (ISO 4287 theoretical Ra),
  `mach_beam_deflection` (Shigley table A-9: cantilever ± end /
  uniform load; simply-supported ± center / uniform load),
  `mach_section_inertia` (rectangle b·h³/12, round π·d⁴/64),
  `mach_stress_strain`, `mach_bolt_torque` (Shigley K-factor table —
  dry as-received K = 0.30 surfaced explicitly; popular 0.20 default
  is for lubricated), `mach_thread_spec` (vendored ASME B1.1 UNC and
  ISO 261 metric coarse tables with 75 % engagement tap drills),
  `mach_material` (MatWeb / ASM yield / ultimate / ρ / E / ν for 1018,
  4140 Q&T, SS 304, 6061-T6, 7075-T6, Ti-6Al-4V, brass C260),
  `mach_hardness_convert` (ASTM E140-12b interpolated HRC ↔ HV ↔ HB).

- **CNC / OpenSCAD family** (`gcode_*`, `scad_*`, 7 tools):
  `gcode_drill_hole` and `gcode_bolt_pattern` (portable
  RS-274/NGC v3 — LinuxCNC dialect; preamble `G17 G21 G90 G94`),
  `gcode_parse_summary` (per-command counts, modal state, axis
  travel, bounding box; handles `( ... )` and `;` comments),
  `scad_box`, `scad_cylinder`, `scad_sphere`, and `scad_flange`
  (idiomatic `difference()` of disc + N-rotated-translated
  cylinders, with the canonical −1/+2 mm Z over-extrusion so holes
  cleanly cut both faces).

- **Smoketest extended to 163/163 tools** — every new chemistry /
  biology / nuclear / radiology / machinist / CNC tool exercised
  end-to-end through `Skill::call` against real JSON args.

### Changed

- **Constellation sharing invariant for scientific data retrieval.**
  All life-sciences REST fetches (`bio_uniprot_get`, `bio_pdb_get`,
  `bio_ensembl_lookup`), live open-data feeds
  (`opensky_states`, `usgs_earthquakes`, `swpc_solar_wind`), and the
  space-weather Kp fetch (`atm_space_weather_kp`) now go through
  `server.retrieval_get` and `server.retrieval_put` so a constellation
  peer that already fetched a record can serve it to the mesh
  without anyone re-hitting the upstream until the cache TTL expires.
  Keys are stable canonical strings — `uniprot|P12345`, `pdb|1HHO`,
  `ensembl|<id>|expand=…`, `opensky|<bbox>`, `usgs_quake|<min>|<period>`,
  `swpc|plasma-1-day`, `swpc|planetary-k-index`.

### Validation

Every formula and constant in the new families was checked against a
primary or canonical source via parallel research **before**
implementation; sources are baked into module top-docs and each tool's
`description()`:

- **CODATA 2022** for u ↔ MeV (`Rev. Mod. Phys.` 97, 025002).
- **AME2020 / NUBASE2020** for nuclide masses + half-lives + decay
  modes (`Chin. Phys. C` 45, 030001 / 030003).
- **Krane** *Introductory Nuclear Physics*, Wiley 1988, §3.3 for the
  semi-empirical mass formula.
- **Bateman** *Proc. Camb. Phil. Soc.* 1910, 15:423 for the two-step
  decay-chain closed form; the L'Hôpital regime for equal rates is
  guarded explicitly.
- **ICRU 85** (dose unit definitions); **NIST W/e = 33.97 J/C** for
  R ↔ Gy_air.
- **ICRP Publication 103** for radiation weighting factors, including
  the piecewise-continuous neutron-w_R function Eq. (B.1.1).
- **ICRP Publication 118** (lens-of-eye limit revision) noted alongside
  US 10 CFR 20 in `rad_occupational_limits`.
- **NIST XCOM** for mass attenuation coefficients; **ORNL/RSIC-45/R1**
  (Unger & Trubey 1982) for specific air-kerma rate constants Γ.
- **NNDC NuDat 3.0** + **IAEA Live Chart** for isotope data.
- **Machinery's Handbook 31st ed.** for cutting kinematics, threads,
  and tap drills.
- **Sandvik Coromant** for the Kienzle k_c1 / m_c coefficients.
- **ISO 4287:1997** for surface-texture parameters.
- **Shigley's** *Mechanical Engineering Design*, 11th ed., Tables A-9
  and 8-15 for beam deflection and bolt nut factors. The Shigley
  K = 0.30 dry value (which differs from the popular 0.20 shorthand)
  is surfaced explicitly in the bolt-torque tool description.
- **ASME B1.1** + **ISO 261** for thread specifications.
- **ASTM E140-12b** Table 1 for hardness conversions.
- **NIST RS-274/NGC v3** for the canonical G-code dialect.

The equation balancer's previously-flagged SVD approach was already
replaced in 0.1.4 with exact rational arithmetic; this release does the
same for the new families' algorithms — every numeric path is either a
closed-form expression with a cited source or a defensively-coded
discrete computation.

## [0.1.4] - 2026-06-02

Chemistry and life-sciences capability release. Every algorithm carries a
citation in its tool description; the implementation was validated against
named primary sources (IUPAC CIAAW, NCBI, Unimod/Expasy, Wallace 1979,
Marmur & Doty 1962, Needleman & Wunsch 1970, Smith & Waterman 1981,
Michaelis & Menten 1913, Hardy 1908 / Weinberg 1908) before landing.

### Added

- **Chemistry family** (`chem_*`, 10 tools): `chem_periodic_table` (118
  elements with IUPAC CIAAW 2021 abridged atomic weights, common
  oxidation states, group/period, category — Lu and Lr placed in Group
  3 per the IUPAC provisional report); `chem_molar_mass` (formula
  parser supports parentheses `Ca(OH)2`, brackets, and hydrates
  `CuSO4·5H2O` / `CuSO4.5H2O`); `chem_formula_hill` (Hill 1900
  ordering); `chem_balance_equation` (**exact integer** stoichiometry
  via fraction-free Gauss-Jordan + LCM/GCD rationalization — no SVD
  round-off, detects infeasibility and under-determined systems);
  `chem_ph` / `chem_buffer` (Henderson-Hasselbalch); `chem_ideal_gas`
  (PV = nRT, R = 8.314 462 618 J/(mol·K), the exact value defined by
  the 2019 SI redefinition); `chem_dilution`, `chem_gibbs`,
  `chem_radioactive_decay`.

- **Biology family** (`bio_*`, 12 tools): `bio_dna_complement` (Watson-
  Crick, reverse by default), `bio_transcribe` (coding-strand default
  with `template` option), `bio_translate` (NCBI standard table 1,
  reading frames 1-3, ambiguous codons → `X`), `bio_gc_content`,
  `bio_codon_lookup`, `bio_protein_mw` (monoisotopic peptide MW using
  Unimod/Expasy reference residue masses + 18.01056 Da water),
  `bio_orf_finder` (complete ORFs in all 6 frames), `bio_pcr_tm`
  (Wallace ≤ 14 nt or basic Marmur 15-50 nt), `bio_align_global`
  (Needleman-Wunsch), `bio_align_local` (Smith-Waterman),
  `bio_michaelis_menten`, `bio_hardy_weinberg`.

- **Bio-data feeds** (`bio_*_get` / `bio_*_lookup`, 3 tools):
  `bio_uniprot_get` (rest.uniprot.org/uniprotkb/{id}.json),
  `bio_pdb_get` (data.rcsb.org/rest/v1/core/entry/{id}),
  `bio_ensembl_lookup` (rest.ensembl.org/lookup/id/{id}?expand=…). All
  keyless live REST.

- **Smoketest extended** to exercise all 22 new chemistry/biology
  tools end-to-end (128/128 tools cover-tested now).

### Validation

Every formula and constant was checked against primary or canonical
sources via parallel research before implementation:

- IUPAC CIAAW *Standard Atomic Weights 2021* for the periodic table
  (Prohaska et al., *Pure Appl. Chem.* 2022, 94(5):573-600).
- 2019 SI redefinition for R = 8.314 462 618 J/(mol·K) (exact).
- NCBI Translation Table 1 for the genetic code, verbatim.
- Unimod / Expasy reference for monoisotopic amino-acid residue
  masses + 18.01056 Da peptide water.
- Wallace et al., *NAR* 1979 6:3543 and Marmur & Doty *JMB* 1962 5:109
  for primer Tm.
- Needleman & Wunsch (*JMB* 1970, 48:443), Smith & Waterman (*JMB*
  1981, 147:195), Michaelis & Menten (*Biochem. Z.* 1913, 49:333),
  Hardy (*Science* 1908, 28:49), Weinberg (1908) for the named
  algorithms.

The equation balancer was specifically rewritten to use **exact integer
arithmetic** (fraction-free Gauss-Jordan over ℤ, then LCM/GCD to
rationalize) after research flagged the original SVD-based approach as
fragile for non-trivial reactions. The new implementation balances
H₂ + O₂ = H₂O → 2 H₂ + O₂ = 2 H₂O, propane combustion C₃H₈ + 5 O₂ =
3 CO₂ + 4 H₂O, blast-furnace iron Fe₂O₃ + 3 CO = 2 Fe + 3 CO₂, and
correctly rejects "H₂ + Cl₂ = H₂O" as infeasible (O has no LHS source).

## [0.1.3] - 2026-06-02

Security patch + cosmetic doc updates. No tool catalog changes.

### Security

- **Drop `rustls-webpki 0.102.8` (4 GHSA advisories).** `rumqttc 0.24` was
  the only consumer of the vulnerable version and pinned `^0.102`. Switched
  `rumqttc` to `0.25` with the `use-native-tls` feature so the MQTT TLS path
  runs through the platform stack (Schannel / Secure Transport / OpenSSL,
  whichever each OS vendor patches). Only `rustls-webpki 0.103.13` remains in
  the lock graph (used by `reqwest` for HTTPS). Advisories closed:
  - **GHSA-82j2-j2ch-gfr8** (high) — DoS via panic on malformed CRL BIT STRING.
  - **GHSA-pwjx-qhcg-rvj4** (medium) — CRLs not considered authoritative by
    Distribution Point.
  - **GHSA-xgp8-3hg3-c2mh** (low) — name constraints accepted on wildcard certs.
  - **GHSA-965h-392x-2mh5** (low) — name constraints for URI names incorrectly
    accepted.

  Note: `lodestone-mcp` never processed CRLs through `rumqttc`, so the
  high-severity DoS was not reachable on our actual code paths. Closing the
  alerts cleanly is still the right move.

### Changed

- **README badges.** Added shields for crate version, license, CI status, and
  the MCP / Rust toolchain target so the project page at a glance shows the
  release line and build health.

### Compatibility

- MQTT brokers presented by a certificate the platform trust store doesn't
  recognize will now fail at the OS level rather than via the vendored Rustls
  root set. Self-signed brokers should either be installed into the OS trust
  store, fronted by an HTTPS terminator with a public cert, or accessed via
  `tcp://` from a trusted network.

## [0.1.2] - 2026-06-02

This is a **capability** release: the catalog jumps from ~300 to **~400 tools**
across 19 new modules covering the math / signal / RF / navigation /
geospatial / chart surface. Everything is pure-Rust, keyless, on by default,
and exposed through the normal `Skill` contract — no new gates, no new
families to enable. CI fixes and a new end-to-end smoketest harness round it out.

### Added

- **Linear algebra** (`linalg_*`, 10 tools): `linalg_solve` (LU), `linalg_lstsq`
  (least squares), `linalg_svd`, `linalg_eigen`, `linalg_qr`, `linalg_inv`,
  `linalg_det`, `linalg_rank`, `linalg_norm`, `linalg_matmul`. Backed by
  `nalgebra` — full dynamic `DMatrix<f64>` / `DVector<f64>`.

- **Quaternion algebra** (`quat_*`, `frame_dcm_from_euler`, 8 tools): Euler ↔
  quaternion, compose/rotate/conjugate/normalize/slerp, plus an Euler →
  direction-cosine-matrix helper. Hamilton convention (`w, x, y, z`).

- **ODE integration** (`ode_rk4`): classical fourth-order Runge-Kutta, with
  per-state RHS expressions compiled once via `meval` and evaluated against a
  `(t, y0, y1, …)` context — good enough for projectiles, oscillators, and
  small ecology / pharmacokinetics models.

- **Geodesy & coordinate systems** (`geo_*`, 12 tools): WGS84 Vincenty
  inverse/direct geodesics, great-circle polyline densify, cross-track distance,
  ellipsoidal polygon area via Karney's `geographiclib::PolygonArea`, UTM
  forward/inverse (Karney series form), MGRS forward/inverse, ECEF ↔ geodetic
  (Bowring's iteration), and a 7-parameter Helmert datum transform.

- **Atmospheric models** (`atm_*`, 5 tools): US-Standard-Atmosphere-1976 layered
  ISA (7 layers, 0–86 km), density altitude with optional humidity correction,
  Magnus-formula dewpoint, Stull wet-bulb / WBGT, and live planetary K-index
  via NOAA SWPC.

- **Information theory + coding** (`it_*`, `code_*`, 9 tools): Shannon-Hartley
  capacity, Rényi-generalized entropy, KL & JS divergence, mutual information
  from a joint distribution, Hamming distance over hex bytes, CRC (8/16/32/64
  variants), Reed-Solomon encode (`reed-solomon-erasure`), K=7 rate-1/2
  convolutional encoder (G1=0o171, G2=0o133).

- **Crypto-math primitives** (`crypto_*`, 9 tools): Miller-Rabin probabilistic
  primality (via `num-bigint::RandBigInt`), big-integer modular exponentiation,
  modular inverse, Chinese Remainder Theorem, HKDF-SHA-256, PBKDF2-HMAC-SHA-256,
  Argon2id (`argon2`), HMAC-{SHA1/SHA256/SHA384/SHA512}, and `jwt`
  decode-without-verification. Educational / math focus — not a TLS stack.

- **RF link engineering** (`rf_*`, 11 tools): two-ray plane-earth path loss,
  Okumura-Hata (150–1500 MHz), COST-231-Hata (1500–2000 MHz), Egli, ITU-R P.676
  atmospheric absorption (simplified O₂ + H₂O lines), ITU-R P.838 rain
  attenuation (k·R^α), Doppler shift, polarization mismatch (linear / circular /
  cross), Fresnel-zone radius, knife-edge diffraction (Lee J(v) approximation),
  and Friis link-budget with kTBF system-noise floor.

- **Radar equation family** (`radar_*`, 7 tools): monostatic + bistatic range
  equations, coherent / non-coherent integration gain with Marcum loss, pulse
  compression gain (τ·B), CA / OS CFAR thresholds, clutter PDF thresholds
  (Rayleigh / Weibull / K-distribution), radar Doppler.

- **DSP extensions** (`signal_*`, 6 new tools): STFT spectrogram (Hann window,
  configurable overlap), FFT-based cross-correlation with peak-lag, FFT-based
  Hilbert transform (analytic signal + instantaneous frequency), real cepstrum,
  BER curves for BPSK / QPSK / M-PSK / M-QAM / FSK over AWGN or Rayleigh
  (erfc via Abramowitz-Stegun), and IQ demod (amplitude/phase/instantaneous
  freq). Builds on the existing `rustfft` integration.

- **Estimation & tracking** (`track_*`, 3 tools): single-step linear Kalman
  filter (predict + update + NIS for chi-squared gating), Hungarian / Kuhn-
  Munkres optimal assignment (`pathfinding::kuhn_munkres`), and 2-D RANSAC
  line fit.

- **Acoustic / underwater** (`acoustic_*`, 5 tools): Mackenzie 9-term sea-water
  sound speed, air sound speed (with optional humidity), Snell's law
  refraction, Thorp absorption + spherical/cylindrical transmission loss, and
  the full sonar equation (SE = SL − 2·TL + TS − (NL − AG) − DT).

- **Navigation aiding** (`nav_*`, 5 tools): GNSS DOP from line-of-sight unit
  vectors (PDOP / HDOP / VDOP / TDOP / GDOP via (HᵀH)⁻¹), Klobuchar ionospheric
  delay (GPS subframe 4), Saastamoinen tropospheric delay, ECEF → local ENU,
  and an IMU drift model combining angle random walk + bias instability +
  scale-factor RSS.

- **Trajectory mechanics** (`traj_*`, 3 tools): projectile-with-drag RK4
  integrator (variable wind, configurable air density), Hohmann transfer
  delta-v + transfer time, and Sutton-Graves stagnation-point reentry heating.

- **Earth models** (`earth_*`, 2 tools): Greenwich / local mean sidereal time
  via Meeus formula 12.4, and a centred-dipole magnetic-declination estimate
  (2025 epoch, linear secular drift). EGM2008 geoid and full WMM coefficient
  evaluation are larger data files — deferred.

- **Optimization & operations research** (`opt_*`, 2 tools): TSP via
  nearest-neighbour seed + 2-opt refinement, and shortest-path via
  `pathfinding::directed::dijkstra`.

- **Open data feeds** (`open_data_*`, 3 tools): OpenSky network aircraft state
  vectors (with optional bounding box), USGS earthquake GeoJSON feeds
  (1.0/2.5/4.5/significant × hour/day/week/month), and NOAA SWPC real-time
  solar wind plasma.

- **Geospatial format converters** (`convert_*`, 3 tools): NMEA-0183 sentence
  decode (GPGGA / GPRMC / GPGSA / GPGSV / GPVTG with XOR checksum
  verification), Cursor-on-Target XML emit (TAK-ingestible event), and
  GeoJSON → WKT for Point / LineString / Polygon / Multi*.

- **Interchange formats** (`interchange_stl_info`): STL binary + ASCII mesh
  probe — triangle count, AABB, surface area, and centroid.

- **Specialist chart types** (`chart_polar`, `chart_smith`, `chart_waterfall`,
  `chart_compass_rose`, `chart_skyplot`, `chart_density_map`, 6 tools): SVG
  generators following the existing `chart_*` family pattern. Polar (antenna
  pattern) and Smith (RF impedance Γ-plane) plots; waterfall and density-map
  heatmaps (viridis colormap + colorbar); compass rose / wind rose; sky plot
  for satellite az/el.

- **Comprehensive end-to-end smoketest** (`src/smoketest.rs`). One
  `#[tokio::test]` that constructs a real `Lodestone`, walks all 106 new tool
  entries with realistic JSON args, and asserts each `Skill::call` returns
  non-empty content. Doubles as a schema-drift canary: any field rename in an
  `*Args` struct that doesn't propagate to callers surfaces as a named
  failure. Runs in ~1 second on a clean build.

### Fixed

- **CI green again.** Restored `cargo fmt --check` + `cargo clippy --all-targets
  -D warnings` across all the new modules — manual `% 2 != 0` → `is_multiple_of`,
  `for i in 0..vec.len()` → `iter().enumerate()`, approximate `0.7071` /
  `1.5708` literals replaced by `std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2}`,
  and a few `vec![…]` test slices flattened to `&[…]`.

### Dependencies

- Added: `nalgebra`, `geographiclib-rs`, `num-bigint`, `num-integer`,
  `num-traits`, `p256`, `crc`, `hmac`, `sha2`, `hkdf`, `pbkdf2` (with the
  `hmac` feature so `pbkdf2_hmac` is in scope), `argon2`, `jwt`,
  `reed-solomon-erasure`, `minilp`, `pathfinding`, `meval`, `gpx`,
  `wkt`, `base64` (already present, now used in more places).

## [0.1.1] - 2026-06-01

### Changed

- **Dashboard removed from the MCP binary.** Earlier builds embedded the
  Nuxt SPA via `include_dir!` and served it at `/dashboard/*` (plus a
  `/` → `/dashboard/` redirect). The dashboard is now strictly a
  **separate service** (`frontend/Dockerfile` → `lodestone-dashboard`
  image, wired into `docker-compose.yml`). The binary serves only
  `/mcp`, `/ws/status`, `/api/settings/*`, `/api/memory/graph`,
  `/constellation/*`, `/health`. Drop the `include_dir` dependency,
  delete `build.rs` (whose only job was making `include_dir!` happy),
  delete the `build-with-dashboard` / `build-with-dashboard-docker`
  Makefile targets, and rewrite `docs/building.md` around the four
  remaining paths (backend-only / compose / standalone dashboard / dev
  HMR). **Breaking** for anyone hitting `http://lodestone-mcp/dashboard/`
  on the binary — it now returns 404. Switch to the dashboard service
  on its own port (`:8001` in the compose stack).

- **Constellation off by default in the shipped TOML.** The Rust struct
  default and `docs/constellation.md` already said off-by-default; only
  `config/06-network.toml` was shipping `[network].enabled = true`.
  Flipped to `false` and rewrote the header to match. Operators who
  joined a mesh on the prior implicit default need to flip
  `[network].enabled = true` explicitly.

### Fixed

- **CI build.** The `cargo clippy --all-targets -- -D warnings` step was
  failing on Linux because of an unused `use std::path::Path;` inside a
  `#[cfg(target_os = "linux")]` helper in `src/skills/sysinfo.rs`
  (introduced in 0.1.0's GPU split). Windows builds didn't see it
  because the whole function was cfg-gated out. Removed the import.

- **Workflows updated to `actions/checkout@v5`** — `v4` was emitting a
  Node 20 deprecation warning ahead of the 2026-09-16 runner removal.

### Changed

- **Destructive-action audit, gap fixes.** Cross-skill audit against golden
  rule 8 (destructive actions never fire unguarded). Closed six gaps where
  side-effecting tools previously ran without a confirmation challenge:

  | Tool | New surface | Lever |
  |---|---|---|
  | `webpage_to_pdf` | guard challenge + path now confined to `[filesystem].roots` (was writing arbitrary paths!) | `[filesystem].allow_destructive` |
  | `mqtt_publish` | guard challenge — publish can drive IoT actuators / smart-home devices | `[mqtt].allow_destructive` (new lever) |
  | `meshtastic_send` | guard challenge — broadcasts on a physical LoRa mesh | `[meshtastic].allow_destructive` (new lever) |
  | `browser_eval` | guard challenge — arbitrary JS execution in a session | `[browser].allow_destructive` (new family) |
  | `browser_persona_reset` | guard challenge — discards persona warm state | `[browser].allow_destructive` |
  | `store_purge` | guard challenge — deletes cached bytes on disk | `[store].allow_destructive` (new lever) |

  Also added the missing `allow_destructive` lever to four families that
  already had a guard challenge but no session-pre-authorize flag
  (previously hard-coded `pre_authorized = false`): `[ffmpeg]`,
  `[spreadsheet]`, `[serial]`, `[printer]`. Each call site now reads
  `server.cfg.<family>.allow_destructive` so the operator can pre-auth a
  long workflow without per-call confirmations. Net: every destructive
  tool in the catalog now both **prompts the user via guard** AND
  **exposes a session-level lever** (golden rule 8).

### Added

- **Package-manager skill (`[packages]`, off by default).** New family
  covering 11 OS / distro package managers — `winget`, `chocolatey`,
  `brew`, `apt`, `dnf`, `yum`, `apk`, `pacman`, `yay` (AUR), `zypper`,
  `pkg` (FreeBSD). One tool per *method* (`package_search`,
  `package_info`, `package_list`, `package_updates`, `package_install`,
  `package_upgrade`, `package_remove`), each takes an explicit `kind`
  argument — the `db_query` pattern from golden rule 9 (different
  *targets*, not different methodologies). Plus `package_managers`
  (read-only) showing which PM binaries are present on `$PATH`.
  Destructive ops (`install` / `upgrade` / `remove`) route through the
  confirmation guard (golden rule 8); `[packages].allow_destructive`
  pre-authorizes for the session. Each destructive command pins down a
  PM-specific non-interactive flag (`--silent` for winget, `-y` for apt
  / dnf / yum, `--noconfirm` for pacman / yay, etc.) so a backgrounded
  call can never hang on a y/N prompt — enforced by unit tests. **No
  `sudo`** — privilege is the operator's choice (passwordless sudo,
  container UID, doas wrapper, …). Family is `Ready` whenever any
  supported PM binary is on `$PATH`; per-call, the wrapper additionally
  checks the specific `kind` and emits a clean "winget isn't installed"
  message when missing.

- **Global `background: bool` argument on every tool.** The dispatch
  wrapper now merges a shared `background` property into every tool's
  exposed schema before publishing it via MCP `tools/list`. When a
  caller sets `background: true` in a `tools/call`, the wrapper spawns
  the skill body into the shared [`TaskRuntime`] and returns a
  `task_id` immediately; progress + completion notifications flow on
  the caller's `_meta.progressToken`. The skill body itself runs
  unchanged — the wrapper strips `background` out of the args before
  calling `Skill::call`.

  Means **any** tool (`shell_run`, `python_run`, `ffmpeg_convert`,
  `arithmetic_eval`, anything) can be backgrounded without per-skill
  code changes. Replaces what would have been a `run_async` wrapper or
  per-tool `*_async` variants. Closer to the MCP spec's intended shape
  (`_meta.taskMode = "augment"`) without depending on rmcp's dispatch
  internals.

  V1 limitations (documented in `route()`): backgrounded calls skip
  the memory-recall preamble and conversation-recording side effects.
  The model can call `recall` / `conversation_show` itself against
  `tasks_result` if needed. Skill ownership in the dispatcher
  switched from `Box<dyn Skill>` to `Arc<dyn Skill>` to support the
  `tokio::spawn` clone.

### Changed

- **Legacy `task_*` (singular) family collapsed; `task_run` renamed to
  `search_async`.** The four management tools (`task_list`, `task_status`,
  `task_result`, `task_cancel`) were near-duplicates of the MCP-spec
  `tasks_*` (plural) tools that read the same registry — deleting them
  removes a confusing parallel surface without losing capability. The
  remaining tool, `task_run`, was always a search launcher (only `op =
  "search"` was supported), so it's now `search_async` — the name says
  what it does. Same runtime, same `task_id` namespace, same gating
  (`[tasks].enabled`). **Breaking** for prompts/clients that hard-code
  the old tool names; switch to `search_async` + `tasks_*`.

- **Legacy `task_*` (singular) tools now ride on `TaskRuntime`.** The
  parallel `skills::tasks::Tasks` polling-registry is gone; `task_run`
  / `task_list` / `task_status` / `task_result` / `task_cancel` are
  thin presentation layers over [`crate::tasks::TaskRuntime`] — the
  same shared registry `mqtt_listen`, `meshtastic_listen`, and the
  MCP-spec `tasks_*` (plural) tools use. One source of truth instead
  of two; `task_*` tool names preserved for back-compat. Bonus:
  `task_run` now emits `notifications/progress` (engine start +
  completion) and `notifications/tasks/status` (lifecycle) when the
  caller's request includes `_meta.progressToken`, matching the
  listen tools' behavior. `server.tasks` field removed; nothing
  outside the legacy module referenced it.

### Added

- **MCP Tasks primitive (2025-11-25 spec), Lodestone-side.** New
  [`crate::tasks::TaskRuntime`] — global, `Arc`-shared registry of
  long-running operations. Any skill can `spawn(kind, label, body)`
  and receive a `task_id`; the body gets a `TaskHandle` for
  `progress(p, total, msg)` calls and a `CancellationToken` for
  cooperative cancellation. Lifecycle transitions emit
  **`notifications/tasks/status`** (the spec's completion-push) and
  per-progress calls emit **`notifications/progress`** (standard MCP),
  correlated by the `progressToken` the caller put in
  `_meta.progressToken`. rmcp 1.7 doesn't ship a typed variant for
  `notifications/tasks/status` yet — we use its
  `ServerNotification::CustomNotification` escape hatch, emitting the
  exact wire bytes a typed variant would; the day rmcp adds one, the
  swap is one type change. Bounded at 256 simultaneous tasks (oldest
  finished evict first); progress log capped at 128 entries per task
  for replay via `tasks_result`.

  Surface (MCP tools, mirror of the spec's `tasks/*` methods so every
  client works today regardless of native Tasks support):
  - `tasks_list` — list tracked tasks (newest first).
  - `tasks_get` — one task's metadata.
  - `tasks_result` — terminal result or in-progress log replay.
  - `tasks_cancel` — fires the cancellation token + status push.

  Internal surface (`SkillCtx`): `peer: Option<Peer<RoleServer>>` and
  `meta: Option<Meta>` now flow through so any skill can extract the
  caller's `progressToken` and register observers on the runtime. The
  dispatch wrapper populates both from the rmcp `RequestContext` of
  each call.

  Distinct from the legacy `task_*` (singular) skill — that one stays as
  a polling-only background search-results buffer. New work uses the
  runtime.

- **`mqtt_listen` and `meshtastic_listen`** — first consumers of the
  new runtime. Each returns a `task_id` immediately, then streams
  per-message progress notifications until `max_messages` /
  `timeout_secs` / `tasks_cancel`. The collected payloads are
  fetchable via `tasks_result`. Demonstrates the pattern: get
  `ctx.peer.clone()` + `ctx.progress_token()`, `runtime.spawn(...)`,
  `runtime.observe_progress(...)` + `runtime.observe_status(...)`,
  return immediately.

- **MQTT pub/sub skill (`[mqtt]`, off by default).** Generic MQTT client
  via `rumqttc` — one persistent connection (background event-loop task)
  with publish/subscribe handles cloned to tool calls and a process-wide
  ring buffer (`[mqtt].buffer_size`, default 500) of inbound messages.
  Tools: `mqtt_publish`, `mqtt_subscribe`, `mqtt_unsubscribe`,
  `mqtt_recent`, `mqtt_status`. Broker URL scheme picks the transport —
  `tcp://` / `mqtt://` for plain, `tls://` / `mqtts://` for MQTTS over
  rustls. `[mqtt].password` is treated as a secret (golden rule 11) —
  redacted to `<set>` / `<unset>` in status, never logged. Auto-subscribe
  list (`[mqtt].auto_subscribe`) for always-on feeds. Per-tool capability
  is `Ready` (network protocol, nothing host-local to probe); wiring
  state surfaces via `mqtt_status` + per-call errors when the broker
  isn't connected.

- **Meshtastic skill (`[meshtastic]`, off by default).** Read / send
  Meshtastic LoRa mesh traffic via the JSON-over-MQTT topic format the
  firmware emits when `MQTT.json_enabled = true`. **Rides on the same
  `MqttClient`** the MQTT family uses — one connection, one event loop,
  one buffer. Auto-subscribes to `<root>/+/2/json/#` at startup so the
  buffer fills without an explicit `mqtt_subscribe`. Tools:
  `meshtastic_messages` (decoded text traffic with `channel` / `from`
  filters), `meshtastic_nodes` (id / longname / shortname / RSSI / SNR /
  last-seen, accumulated from the buffer), `meshtastic_send` (formats
  the `sendtext` envelope and PUBLISHes), `meshtastic_status`. Serial /
  TCP / BLE transports + protobuf decode are a deferred follow-up.
  Per-tool errors guide the LLM when `[mqtt]` isn't wired up.

### Changed

- **`FamilyMeta::description()` and `FamilyMeta::check_capability()` are now
  required.** Their previous defaults (`""` and `Ready` respectively) were a
  contract leak: implementing `FamilyMeta` is itself an assertion that the
  dashboard, operator, and dispatch wrapper should care about this family —
  and the two methods that make the framework care can't meaningfully be
  skipped. Pure-Rust families that have no host probe simply don't register
  `FamilyMeta` and are treated as implicitly `Ready` in dispatch (unchanged).
  All 9 existing impls already supplied both, so no behavior change.
  `Skill::check_capability()` keeps its `Ready` default — there the default
  carries real meaning ("this tool is pure logic, nothing host-dependent")
  for the hundreds of pure-Rust skills that genuinely don't need a probe.

- **Dashboard surfaces family + tool descriptions.**
  Adds `FamilyMeta::description()` (with a `""` default for incremental
  adoption) and wires `Skill::description()` through the WebSocket
  snapshot so the dashboard's Tools page renders a one-line family blurb
  under each group header and the tool's own description under each tool
  name. The `ServerStatus.tool_descriptions` row covers active + config-
  gated tools alike. The 9 existing `FamilyMeta` impls (docker, kubernetes,
  python, systemd, ffmpeg, git, serial, printer, sdr) now supply a real
  description. Note: `FamilyMeta::description` is dashboard-only —
  `Skill::description` is what the LLM actually reads via the MCP
  `tools/list` response.

- **`TOOL_NAMES` per-module consts removed; derived from `skills()`.**
  The 30+ `pub const TOOL_NAMES: &[&str] = &[…]` declarations and the
  `FamilyMeta::tools() -> &'static [&'static str]` signature were a
  hand-maintained duplicate of the same module's `skills()` registry
  (every `Skill::name()` is already `&'static str`). The trait now
  returns `Vec<&'static str>` and the 9 `FamilyMeta` impls all compute
  it as `skills().iter().map(|s| s.name()).collect()` — boxes are
  constructed once at startup and dropped after the names are extracted.
  `disabled_by_config` follows the same pattern (calling each module's
  `skills` directly). Single source of truth: the `skills()` `vec!`
  literal. Adding a tool now only requires the one `Box::new(...)` line.

- **`system_gpu` split into three per-vendor tools.** Replaces the single
  `system_gpu` with `system_gpu_nvidia`, `system_gpu_amd`, and
  `system_gpu_intel`. Each is its own `Skill` with its own
  `check_capability`:
  - `system_gpu_nvidia` — NVML via `nvml-wrapper`, cross-platform; capability
    flips Ready iff `Nvml::init()` succeeds.
  - `system_gpu_amd` — reads `/sys/class/drm/card*/device/` nodes the
    `amdgpu` Linux kernel driver publishes (model, VRAM, busy %, hwmon
    temperature); capability requires Linux + at least one card with PCI
    vendor `0x1002`.
  - `system_gpu_intel` — same DRM sysfs surface for `i915` / `xe` (model,
    frequency, hwmon temperature); capability requires Linux + PCI vendor
    `0x8086`.

  Driver for the split is golden rule 9 (one tool per method, no hidden
  auto-selection): NVML and the DRM sysfs path are genuinely different
  backends with different failure modes, and the per-tool capability
  framework already in place gives each its own Ready / Unavailable signal
  surfaced to LLM, console, and dashboard. AMD / Intel on Windows / macOS
  remain unsupported (would need ADL / IGCL / IOKit), which is now an
  explicit per-tool Unavailable rather than a vague combined "no GPU
  detected" message.

- **Constellation merge rule: prefer the larger mesh.** When two
  constellations meet, the **larger** mesh's id wins so the smaller mesh
  adopts the larger one (alphabetical id is now only the tiebreaker on
  equal sizes). The prior rule — "smallest id wins regardless" — was
  deterministic but semantically arbitrary: a 50-node mesh could be
  forced to adopt the id of a 2-node mesh just because the small mesh
  happened to have an `"aaa-…"`-style id. The new rule prefers the
  more-defined mesh, matching the intuition that "the bigger thing
  absorbs the smaller thing." Implementation: `Digest` gains
  `peer_count: usize` carrying the FULL count of reachable peers (the
  existing `peers` field is a gossip *sample* capped at
  `MAX_GOSSIP_PEERS = 64`, so wasn't usable for size comparison once a
  mesh got past 64 nodes); `maybe_adopt_id(peer_cid, peer_peer_count)`
  compares mesh sizes first, falls back to alphabetical id on tie.
  Backward compat: `serde(default)` on `peer_count` so older peers
  default to 0 — they'll lose every merge against a newer peer (safe
  default; they're either alone or upgrading).

  **Propagation.** Each adopt is one hop; the change spreads through the
  connected mesh via the normal gossip path. When node X adopts, X's
  next digest carries the new id, X's other peers see it on their next
  sync and run the same rule. Full convergence in
  `O(sync_secs × diameter)` — a few minutes for typical sparse meshes
  with the default 30s sync. Documented under §"Auto-healing on network
  change" in `docs/constellation.md`.

### Added

- **Retrieval delegation** — opt-in "go fetch this URL for me" service over
  the constellation, with cross-constellation transfer via the existing
  galaxy peering. Closes the rate-limit gap: when local cache + peer-cache
  both miss and the direct upstream fetch fails (429 / blocked /
  captive-portalled), a peer in the same or a peered constellation can
  perform the fetch on the consumer's behalf and the rate-limited upstream
  is hit **once for the mesh** instead of once per node.
  - **`POST /constellation/retrieve`** (`src/main.rs`, served by
    `Constellation::serve_retrieve` in `src/constellation/mod.rs`). Body:
    `{ url, max_bytes, source }`. Gated by `[network].token` AND
    `[network].delegation_enabled = true` (off by default — never serve
    outbound traffic for someone else without choosing to). The serving
    node fetches from upstream, caches the body in
    [`IndexedRetrievalCache`](src/retrieval.rs) under the requester-
    supplied `Source` (so the mesh now has it cached behind the digest
    Bloom for everyone), and streams the bytes back. The requester
    identifies itself via `X-Lodestone-Peer-Id: <node_id>` so the
    sliding-window rate limiter can account per-peer.
  - **`Constellation::delegated_fetch`** — the client side. Walks
    Bloom-reachable peers that advertised `delegation_enabled = true` on
    their most recent digest (sorted by reputation, capped at
    `max_peers`), POSTs `/constellation/retrieve` to each in turn, and
    returns the bytes from the first successful response. 429 / 403
    responses log the reason at debug.
  - **`Digest.delegation_enabled`** — peers carry their opt-in flag in the
    digest so requesters only contact willing servers. `serde(default)`
    keeps older peers (which omit it) defaulting to `false`.
  - **`fetch_bytes_shared`** lookup order now: local store → peer cache
    via `consult_blob` → direct upstream → **delegated peer fetch**. The
    delegation step only fires on direct-fetch failure (so a happy path
    doesn't even touch peers); on success the bytes also land in the
    local file store. With no peers configured this is just a plain HTTP
    download — no behaviour change for single-node setups.
  - **Cross-constellation transfer** works automatically through the
    galaxy broker's existing directory model. Foreign-constellation
    endpoints added by `galaxy::client::sync_once` get their digests
    fetched on the next sync, their `delegation_enabled` becomes visible,
    and `delegated_fetch` walks all peers regardless of constellation
    origin. The broker itself is **not a proxy** — bytes flow direct,
    constellation-to-constellation. No new galaxy endpoints needed.
  - **Sliding-hour-window rate limiter** (`src/constellation/delegation.rs`)
    with four knobs:
    - `delegation_max_jobs_per_peer_per_hour` (default 30) — caps how
      many delegated fetches any single peer can request per hour.
    - `delegation_max_bytes_per_job` (default 8 MiB) — caps the body size
      of a single fetch.
    - `delegation_total_bytes_per_hour` (default 256 MiB) — global
      aggregate-byte cap. Protects local egress.
    - `delegation_max_cache_bytes` (default 64 MiB) — caps the summed body
      size of all retrieval-cache entries (delegated or not). Eviction is
      oldest-by-expiry. 0 = unlimited.
    Rejected requests return HTTP 429 (peer-jobs / global-bytes) or 413
    (per-job size) with a JSON `RetrieveReject { reason, retry_after_secs,
    detail }` body + a `Retry-After` header so requesters can back off
    intelligently. The limiter uses reservation slots that roll back on
    failed fetches (a `Drop` impl returns the reservation to the budget),
    so a bad URL doesn't permanently burn the requester's quota.
  - **Cache byte budget on `IndexedRetrievalCache`** — added third
    constructor argument `max_bytes`. The cache now tracks summed body
    size + evicts oldest-by-expiry when the cap would be exceeded by a
    `put`. Single bodies larger than the whole cap are refused outright
    rather than evicting the entire cache for an entry that wouldn't fit.
    0 = unlimited (matches prior behavior).
  - All env overrides via `LODESTONE_NETWORK_DELEGATION_*`.

  See [`docs/constellation.md`](docs/constellation.md) §"Retrieval
  delegation (opt-in)" for the lookup order, the guardrail table, the
  cross-constellation transfer flow, and the privacy posture (the URL
  crosses the wire — that's why it's opt-in for the serving side).
  7 new unit tests for the rate limiter (disabled rejection, per-job
  byte cap, per-peer jobs cap, global bytes cap, slot rollback on drop,
  commit-updates-actual-bytes, retry-after hints); 3 new unit tests for
  the cache byte budget (eviction-oldest-first, oversized-body refusal,
  zero-means-unlimited).

- **Multi-identifier retrieval cache + per-source consensus policy** for the
  constellation. Closes the alignment gap that made the mesh useless for
  long-tail rate-limited content (a specific arXiv paper, a specific Wayback
  snapshot — usually only one peer in the mesh has it, so the existing
  `min_agreement = 2` consensus floor always failed and control fell back to
  the rate-limited source).
  - **`Identifiers`** (`src/constellation/identifiers.rs`) — each cache entry
    now declares **every public name it's known by**: the canonical primary
    key, URL aliases (raw URL, resolved snapshot URL, redirect target),
    source-specific identifiers (`("arxiv", "1706.03762v5")`,
    `("wayback_ts", "20240315120000")`, `("doi", "10.48550/…")`), and the
    body's content hash. Built with a small fluent builder
    (`Identifiers::new(key).with_source(Source::…).with_url(…).with_source_id(…, …)`).
    Capped at 8 identifiers per entry to keep the digest small.
  - **`Source`** enum classifies the upstream (`Wayback` / `Arxiv` / `Github` /
    `Overpass` / `SearchEngine` / `Other`) and drives two per-source policies:
    a TTL override (Wayback / arXiv / GitHub-release = 7 days, Overpass = 1
    day, search engines = 1 hour) and a `min_agreement` floor (1 for
    content-addressable, max(cfg, 2) for volatile).
  - **`IndexedRetrievalCache`** (`src/retrieval.rs`) replaces the single-
    keyed retrieval `TtlCache`. Multi-index: every identifier hash is a
    secondary lookup that resolves to the same entry. `lookup_by_hash(h)`
    walks the index in one mutex grab. Eviction (LRU-by-expiry under the
    size cap) sweeps every secondary mapping in one pass. In-memory only for
    v1 (Redis multi-key with atomic secondary-index updates is deferred —
    single-node deployments use this; multi-node deployments share via the
    constellation).
  - **`Lodestone::retrieval_lookup(ids)`** / **`retrieval_put_indexed(ids,
    body)`** — the new public API; existing
    `retrieval_get(key)` / `retrieval_put(key, body)` are now thin shims that
    wrap a single primary key, so every existing skill keeps working
    unchanged. The shims store under `Source::Other` (global TTL + global
    `min_agreement`), so semantics are preserved.
  - **`Constellation::consult_blob_hash_sourced(hash, source)`** — the per-
    source consensus path. For content-addressable sources
    (`Wayback`/`Arxiv`/`Github`) the `min_agreement` floor drops to 1
    *regardless* of `[network].min_agreement`: the safety comes from the
    consumer-side bytes-hash check (step 3 of the anti-tampering flow), not
    from peer count, so requiring N peers to corroborate a hash a single
    peer derived from the same identifier the consumer was looking up by
    adds latency without adding safety. For volatile sources the existing
    multi-peer corroboration applies, and a user hardening to
    `min_agreement = 3` is never silently relaxed. `consult_blob_hash`
    (without source hint) delegates to the sourced path with `Source::Other`,
    so existing call sites land on the global policy.
  - **First adopter — Wayback** (`src/skills/archive.rs`). On lookup the
    skill builds `Identifiers { primary_key, source: Wayback, urls: [raw_url],
    source_ids: { "wayback_ts": timestamp } }` so a peer that cached the same
    `(url, timestamp)` snapshot under a different `max_chars` key still
    serves the consumer. On store, the resolved snapshot URL is added as a
    second URL alias and the captured 14-digit timestamp is auto-extracted
    from the snapshot URL — so an entry self-attaches its full identifier
    set even when the caller didn't supply a timestamp.

  See [`docs/constellation.md`](docs/constellation.md) §"Multi-identifier
  retrieval entries" and §"Per-source consensus policy" for the per-source
  policy table. 22 new unit tests covering put/lookup by every identifier,
  per-source TTL override application, secondary-index eviction, primary-
  key overwrite cleanly removing prior aliases, content-hash auto-attachment,
  and the identifier cap.

  **Adopters** (this commit pulse — every rate-limit-painful source now
  classifies its cache entries):
  - **`arxiv_get`** (`src/skills/arxiv.rs`) — `Source::Arxiv`; lookup carries
    `("arxiv", "<id>v<ver>")` source-id, store adds abs URL + PDF URL aliases.
    arXiv's 3-second-gated API now serves the mesh from any node that has
    the paper, regardless of whether the consumer asks by id, abs URL, or
    PDF URL.
  - **`arxiv_search`** — primary key only (search results aren't
    content-addressable by paper id), keeps `Source::Other` semantics.
  - **`github_releases`** (`src/skills/github.rs`) — `Source::Github`; URL
    aliases include the listing page and each release's canonical `html_url`
    up to the 8-identifier cap, so a consumer probing a specific
    `/releases/tag/<tag>` URL hits this entry.
  - **`grid::run_overpass`** + **`osm_overpass`** — `Source::Overpass`; both
    sites attach `("overpass_qhash", hash_of_QL)` so a query run under any
    skill is reachable from any other. Cross-skill cache hits for free.
  - **Search-result consensus** (`src/constellation/mod.rs::consensus`) —
    inherently `Source::SearchEngine`; the effective `min_agreement` floor
    is now `max(cfg, 2)` regardless of `cfg.min_agreement`, so a user that
    drops cfg to 1 (for some other consult path) doesn't accidentally
    accept lone-peer search results.

### Documentation

- **[`Makefile`](Makefile)** wraps the pre-commit triad and the CI gate as
  named targets. `make check` runs `fmt + build + clippy + test` (golden rule
  10); `make ci` runs exactly what CI runs (`fmt --check` instead of `fmt`);
  `make docker` builds the image and runs the same `/health` smoke test the
  CI `docker` job does; `make install-hooks` drops a `.git/hooks/pre-commit`
  wrapper around `make ci`. `make help` (the default target) shows every
  available target self-documenting from its `## …` comment. Mirrors
  `.github/workflows/ci.yml` so locally-green ≈ CI-green.
- **Two new golden rules** ([`docs/golden-rules.md`](docs/golden-rules.md)).
  - **Rule 10:** `cargo fmt` and `cargo clippy --all-targets -- -D warnings`
    must pass before every commit (CI enforces both). The expanded
    [`CONTRIBUTING.md`](CONTRIBUTING.md) "Build & verify" section explains
    why deny-warnings, what each flag means, common pitfalls, and editor /
    pre-commit-hook integration (rust-analyzer, RustRover, cargo-watch).
  - **Rule 11:** sensitive information must never be shared. Credentials,
    PII, and other secrets are never logged, returned in tool responses,
    committed to git, advertised over the constellation, or echoed back —
    with `[github].token` / `[eia].key` / `[network].token` style
    `<set>` / `<unset>` redaction as the load-bearing example pattern.
    Bearer-token comparisons go through `ct_eq` (`util.rs`) for constant-
    time check.
- **Five mermaid diagrams in [`CONTRIBUTING.md`](CONTRIBUTING.md)** —
  architecture overview, adding-a-skill flowchart, anatomy-of-a-tool-call
  sequence, provider-tier decision tree, and the shared-helpers map. Plus
  a corrected "Adding a skill" section (the old "Adding a tool" was
  factually wrong — it described a `#[tool_router]` flow that contradicts
  golden rule 7). New "Shared helpers" section tables every utility now
  in `crate::util`, `crate::skills`, and `crate::config` with what each
  one replaces.
- **20 new per-skill detail pages** under
  [`docs/skills/`](docs/skills/) — chart, html, image, fcc, binary,
  signal, wave, pcap, disasm, notebook, python, systemd, weather, noaa,
  osm, grid, peeringdb, yahoo, eia, astro, radio. Each follows the
  existing template (header table → "What it does" → tool list → example
  uses → notes → "See also"). [`docs/skills.md`](docs/skills.md) gained
  rows for every previously-undocumented family;
  [`docs/tools.md`](docs/tools.md) was reorganized so the chart / html /
  image / fcc / weather / geo / binary-analysis / runtime / energy /
  astro / radio tools sit under their own headings rather than under
  "Meta".

### Internal — shared helpers (audit-driven WET-code reduction)

A pass across the codebase pulled repeated patterns into shared modules so
adding a new skill is now mostly a wiring exercise rather than a copy-and-edit
exercise. None of this changes runtime behavior; every change kept all tests
green and clippy clean. **Contributors:** see [`CONTRIBUTING.md`](CONTRIBUTING.md)
§"Shared helpers" for the full inventory and when to use which one.

- **`crate::skills::send_json` / `send_json_ctx`** (`src/skills/mod.rs`).
  Centralizes the `.send().await.map_err…error_for_status().map_err…json().await.map_err`
  ritual that 6+ skill modules repeated at every API call. `send_json_ctx`
  prefixes every error with a uniform label (`"open-meteo: …"`, `"nws …: …"`)
  so a single string controls error formatting across all three failure sites
  (network / status / decode). Adopted by `weather`, `noaa`, `peeringdb`,
  `eia`, `grid`, `osm` — each lost a hand-rolled 8-10 line `fetch` helper.
- **`crate::skills::fs_read_bytes`** (`src/skills/mod.rs`). Resolve a path
  against `[filesystem].roots` and read it into bytes, with uniform
  `read <path>: <err>` error formatting. Used by every read-a-file skill
  (`binary`, `image`, `disasm`, `notebook`) — they used to each carry an
  identical hand-rolled `read_file` helper.
- **`crate::skills::live_http`** + **`crate::LODESTONE_UA`** constant. The
  shared User-Agent (`lodestone-mcp/0.1.0 (+https://github.com/…)`) and the
  `cfg(test)` HTTP-client builder collapse what used to be 30+ string literals
  and 28 copies of the same `reqwest::Client::builder()…unwrap()`.
- **`crate::util::url_enc`** (`src/util.rs`). The RFC-3986 unreserved-character
  percent-encoder used to live in 8 skill files (`weather`, `peeringdb`,
  `eia`, `grid`, `osm`, `huggingface`, `yahoo`, `satellite`) as byte-identical
  copies under three different names (`url_enc` / `url_encode` /
  `urlencoding`). One canonical implementation, -77 lines net.
- **`crate::util::human_size`** adopted by `ffmpeg` (drops its byte-identical
  `fmt_bytes` copy). Future size-printing skills no longer have a template
  to copy from.
- **`crate::skills::chart::PlotArea`** + helpers (`svg_open_dark`,
  `title_suffix`, `parse_xy`, `fmt_ts`). Six chart tools (`chart_line`,
  `chart_bar`, `chart_scatter`, `chart_histogram`, `chart_candlestick`,
  `chart_grafana`) now share axis layout + scale math; the title-formatting
  idiom that appeared 12× inline is one call. `parse_xy` + `fmt_ts` are why
  `chart_line` / `chart_scatter` / `chart_grafana` now accept ISO-8601 date
  strings as x values without each tool re-implementing the parse.
- **`crate::skills::meta::family!` macro**. The 23 of 32 `Family { … }` entries
  in `meta::families()` that follow one of two shapes (plain on/off, or
  on/off + `allow_destructive`) now collapse to a one-call
  `family!("<name>", "[<name>]", "…", &["<name>_"], <field>[, destructive])`.
  Adding a new skill family to the `features` introspection tool is now a
  single line.
- **`crate::config::env_apply_{str,bool,parse}`** (`src/config.rs`). The
  `apply_env` block — 417 lines of nearly-identical `if let Ok(v) =
  std::env::var(KEY) { self.foo = …; }` snippets for ~95 settings — now
  reads as one call per setting. `env_apply_parse` is generic over any
  `FromStr`, so a new numeric / float override is one line instead of five.
  Net -91 lines on `config.rs`.
- **`crate::skills::ensure_min_len`** standardizes the "needs at least N
  &lt;what&gt;" invalid-input check used by chart / signal / forecast tools.

### Added

- **`html_render` skill** (`src/skills/html.rs`, on by default via
  `[html].enabled`). Renders an HTML snippet OR navigates to a URL in
  the same shared headless Chrome the rest of the project uses
  (`render_page`, the Google search-provider rendering), waits a
  configurable `wait_ms` for JavaScript to run, then returns a
  structured diagnostics report:
  - **Console** — every `console.log / info / warn / error / debug /
    trace / dir / table / count / time / group / clear / assert / profile`
    call. Level, concatenated args, source URL + 1-based line number from
    the CDP stack-trace top frame.
  - **JS exceptions** — every `Runtime.exceptionThrown` event, with text,
    source / line / column, and a flattened multi-frame stack trace.
  - **Network failures** — every `Network.loadingFailed` event (DNS,
    connection refused, CORS block, ad-blocker interception, mixed-
    content block, …). Distinguished from HTTP errors because no
    response was ever received.
  - **HTTP errors** — every response with status ≥ 400, with URL,
    status, and resource type.
  - **Summary** — final page title, final URL after redirects, total
    elapsed time.

  Use after `chart_interactive` (or any HTML-emitting tool) to verify
  the output actually runs cleanly before shipping it.

  The `PageRenderer` trait gained a `render_diagnostics(input, wait_ms)`
  method; `RenderInput` is a new public enum (`Url` or `Html`). The
  implementation subscribes to CDP `Runtime.consoleAPICalled` /
  `Runtime.exceptionThrown` / `Network.loadingFailed` /
  `Network.responseReceived` event streams BEFORE navigation so early
  events aren't lost to a startup race, collects them into
  `Arc<Mutex<Vec<…>>>` buffers via spawned tasks during the wait, then
  drains and closes. Gate via `LODESTONE_HTML_ENABLED`.

- **Image forensics + EXIF skills** (`src/skills/image.rs`, on by default
  via `[image].enabled`). Four read-only tools, all paths confined to
  `[filesystem].roots`:
  - **`image_info`** — format / dimensions / color / animation from the
    container's structural headers (JPEG SOFn, PNG IHDR, GIF LSD, WebP
    VP8/VP8L/VP8X, BMP DIB, TIFF magic, HEIF brand, JPEG-XL signature).
    Pure binary parsing, no full-image decode.
  - **`image_exif`** — full EXIF tag dump from IFD0 / Exif / GPS /
    Interop via `kamadak-exif`. GPS coordinates are decoded to signed
    decimal degrees with an OSM map link. **Forensic divergence flags**
    fire when `DateTimeOriginal` / `DateTime` / `DateTimeDigitized`
    disagree (re-save / scan workflow indicator) or when the `Software`
    tag is editor-branded (Photoshop / GIMP / Lightroom / Capture One /
    Affinity / Pixelmator).
  - **`image_jpeg_analyze`** — walk every JPEG marker: APP segments by
    identifier (JFIF / Exif / XMP / ICC_PROFILE / MPF / Photoshop / Adobe),
    DQT (quantization tables — encoder fingerprint), DHT counts, DRI,
    SOFn payload (dims / depth / components), SOS. Useful for
    camera-vs-editor source identification and tamper checks.
  - **`image_png_analyze`** — walk every PNG chunk with decoded payloads:
    IHDR, tEXt / iTXt / zTXt (textual metadata — software, comments),
    eXIf, iCCP, tIME, pHYs (with DPI conversion), gAMA, sRGB, acTL
    (APNG animation control). Flags unknown private chunks.

  New dependency: `kamadak-exif = "0.6"` (pure Rust, no native deps).

### Changed

- **`chart_line` accepts ISO-8601 date strings as x values.** A point's
  `x` can now be a number *or* a string like `"2026-01-15"` /
  `"2026-01-15T12:34:56Z"` / `"2026-01-15 12:34:56"`. Strings are
  auto-parsed to Unix timestamps for scaling and the x-axis is rendered
  with date-formatted tick labels (rotated 30° for legibility) instead
  of raw numbers. Closes the "stock chart fails because dates aren't
  numbers" usability gap.

- **Chart / plot rendering skills** (`src/skills/chart.rs`, on by default via
  `[chart].enabled`). Pure-Rust SVG generation — no external dependencies,
  no headless browser, no network. Ten tools total:
  - **`chart_line`**, **`chart_bar`**, **`chart_scatter`**,
    **`chart_histogram`**, **`chart_pie`** — the matplotlib / pyplot
    equivalents. Multi-series line gets a tab10 palette and a legend; the
    histogram auto-bins (√n) when `bins` is omitted; the scatter takes an
    optional point size.
  - **`chart_heatmap`** — 2D matrix as colored cells with a built-in
    colorbar. Colormaps: viridis (default, perceptually uniform), magma,
    plasma, coolwarm (diverging, good for signed data), grayscale. Covers
    correlation / confusion / attention / image-intensity visualizations.
  - **`chart_grafana`** — dark-themed time-series panel with translucent
    area fills, last-value labels, low-contrast grid. For when "this is
    operational telemetry" needs to read at a glance.
  - **`chart_stat`** — Grafana Stat panel. Big-number tile, threshold-
    tinted, with an optional background sparkline. `color_mode=
    "background"` flood-fills the tile for the dramatic green/yellow/red
    status look.
  - **`chart_gauge`** — Grafana radial gauge (270° dial). Threshold bands
    color the arc; numerical readout in the middle.
  - **`chart_bar_gauge`** — Grafana horizontal threshold bars. One row
    per item, value mapped to fill proportion, color from highest reached
    threshold. The Top-N hosts / pods tile.
  - **`chart_state_timeline`** — Grafana State timeline. Categorical
    state bands over time per row — the "is each service up" grid for
    SLO reporting. Sensible state→color defaults (up=green,
    degraded=yellow, down=red, scheduled=blue, unknown=gray); overridable
    via `state_colors`.
  - **`chart_candlestick`** — Grafana Candlestick. OHLC with green/red
    bodies + wicks. Financial time-series.
  - **`chart_sparkline`** — tiny inline trend, no chrome. The shape
    Edward Tufte popularized; Grafana embeds it inside the Stat panel.
    Useful in tables and tight UIs.
  - **`chart_canvas`** — turtle / Logo / matplotlib.patches procedural
    drawing. Issue a sequence of `line`, `rect`, `circle`, `polygon`,
    `polyline`, `text` commands; the tool emits a self-contained SVG.
  - **`chart_interactive`** — wraps Chart.js or Plotly. Returns a
    self-contained HTML snippet that loads the library from a CDN and
    renders the supplied native config. Clients that render HTML get full
    interactivity (hover tooltips, zoom, pan, legend toggling, responsive
    resize); clients that show only text/images see the source.
  - **`chart_mermaid`** — wraps user-supplied mermaid source in a markdown
    code fence. Every modern MCP client (Claude Code, LM Studio, Cursor)
    renders ```mermaid blocks natively, so no server-side rasterization is
    needed and the result re-themes / scales with the client.

  All static outputs are SVG with a `viewBox`, so they scale to the
  renderer's viewport — responsive layout without JavaScript. SVG is
  delivered as MCP `image/svg+xml` content (clients render inline) plus a
  one-line text fallback (clients that don't render images get a
  description). Charts gate via `LODESTONE_CHART_ENABLED`.

- **FCC / amateur radio reference skills** (`src/skills/fcc.rs`, on by default
  via `[fcc].enabled`). Three tools:
  - **`fcc_callsign { callsign }`** — US amateur callsign lookup via the
    keyless callook.info JSON API. Returns licensee, operator class
    (Technician / General / Amateur Extra), trustee for club calls, grant /
    expire / last-action dates, FRN, mailing address, and grid square.
    Switched from data.fcc.gov ULS (HTTP/2-flaky from many networks) to
    callook.info for reliability; non-amateur callsigns (GMRS WQ*/WR*,
    commercial, broadcast) get a friendly ULS web-search hint.
  - **`fcc_amateur_bands { band?, license_class? }`** — full US amateur
    band plan from 2200m through 1.25cm (24 bands total) with
    per-license-class privileges baked in. `band` matches wavelength
    label (`40m`, `70cm`), region (`HF`, `VHF`), or a frequency in MHz
    (`14.250` → 20m); `license_class` filters to Technician / General /
    Amateur Extra.
  - **`fcc_radio_service { service?, channel? }`** — non-amateur personal
    radio services (FRS / GMRS / MURS / CB) regulatory and channel
    reference. Channel maps with frequencies and power caps; how FRS and
    GMRS share spectrum (14 shared channels with different power limits);
    license / antenna / repeater rules per service. `service="compare"`
    for the side-by-side table.

- **`features` introspection tool** (in `src/skills/meta.rs`). Per-family
  enabled/disabled status across every gateable family (memory, constellation,
  filesystem, shell, git, docker, kubernetes, systemd, python, sysinfo,
  databases, serial, printer, sdr, ffmpeg, signal, wave, binary, pcap,
  disasm, notebook, store, tasks, stocks, nasa, eia, github, search), with
  the resolved knob values that control each (`allow_destructive`,
  `recall_threshold`, `embedding_endpoint`, retention policy, …) and live
  counts from the memory store (memos, solutions, embedded ratios, links,
  tags, phrasings, conversations, turns). `features` alone dumps everything;
  `features name="<family>"` focuses on one. The model can ask "is X
  available?" without having to try-and-fail a tool call. Implementation
  needed propagating the full `Arc<config::Config>` onto `Lodestone` (and a
  precomputed `disabled_tools` list) so the tool can answer authoritatively
  without re-resolving config at call time.

- **Dockerfile copies `docs/` and `migrations/` into the build context** so
  the `include_str!` references for `docs/instructions.md` and
  `migrations/*.sql` resolve at compile time. The runtime image still skips
  them — once the bytes are baked into the binary they don't need to ship
  alongside it.

- **Auto-aliasing on semantic-only recall hits** (`[memory]`). When the top
  preamble hit fires only because the embedding cosine cleared the recall
  threshold (token-overlap path didn't), the dispatch wrapper now attaches
  the query as a new phrasing on that solution automatically. Result:
  future token-shaped recall finds the same solution without re-running
  embeddings, and the recall layer's hit rate grows with use. The preamble
  shows a visible `✎ noted this phrasing on the solution for next time` so
  the model can see the system is learning. Guarded by
  `auto_alias_min_query_tokens` (default 3) to stop a single common noun
  from attaching itself to whichever solution it semantically lands on.
  Set `auto_alias_on_semantic_recall = false` to require every alias to be
  attached by explicit `solution_alias_add` call.

- **Semantic recall via OpenAI-compatible embeddings** (`[memory]`). When
  `embedding_endpoint` is set (LM Studio serves one at
  `http://127.0.0.1:1234/v1/embeddings`), every recorded solution and every
  attached phrasing is embedded at write time and stored as a length-
  prefixed `f32` BLOB. `auto_recall` takes `max(token_score, semantic_score)`
  per solution so a question worded with completely different vocabulary
  still surfaces the prior solution. Cosine similarity above
  `embedding_threshold` is linearly mapped onto a token-comparable score
  range; defaults: `0.55` threshold, `text-embedding-nomic-embed-text-v1.5`
  model. Off when `embedding_endpoint` is empty (no network dep). Failures
  degrade silently — the row lands with `embedding=NULL` and re-embeds on
  the next `solution_update`. Migration `0003_embeddings` adds the column.
- **Per-solution phrasings** (`solution_alias_add` / `solution_alias_remove`).
  Lets a solution accumulate multiple ways the same underlying question has
  been asked. Each phrasing carries its own canonical / concept keys (for
  token overlap) and its own embedding (for semantic match); recall scores
  against the union of the solution's own problem text and every attached
  phrasing, taking the best match. Closes the "we'll only ever recall this
  in the original wording" failure mode — over time the recall layer grows
  more robust as the model attaches the alt-phrasings it notices.
  Migration `0003_embeddings` adds the `solution_phrasings` table.

- **Memory levers exposed as config** (`[memory]`): every behavior that used to
  be a hardcoded constant is now a knob. `auto_recall`, `recall_threshold`,
  `recall_max_hits`, `superseded_walk_max_hops` shape the intrinsic-recall
  preamble; `record_conversations`, `conversation_idle_gap_secs`,
  `conversation_turn_excerpt_max_chars`, `record_only_query_calls` tune
  conversation tracking; `conversation_retention_days`, `max_conversations`,
  `prune_on_startup` drive retention. Each has an
  `LODESTONE_MEMORY_<UPPER_SNAKE>` env override.
- **`conversation_forget { id, confirm?, trust? }`** — delete one recorded
  conversation. CASCADE drops its turns; `solution_revisions.conversation_id`
  is set to NULL for any revision that referenced it, so revision content
  remains queryable via `solution_show`. Destructive, guarded.
- **`conversation_prune { older_than_days?, keep_newest?, dry_run?, confirm?, trust? }`** —
  bulk delete by retention policy. Falls back to the configured
  `[memory].conversation_retention_days` / `max_conversations` when neither
  argument is set. `dry_run=true` reports what *would* be deleted without
  asking for confirmation — use to validate the policy first.
- **Startup pruning**: when `[memory].prune_on_startup = true`, the configured
  retention policy is applied once at boot. Off by default so a misconfigured
  policy can't surprise-delete history on first upgrade.

- **Conversation tracking** (when `[memory]` is on): the dispatch wrapper now
  records one row per tool call into a new `conversation_turns` table, grouped
  into `conversations` by a 30-minute idle-gap heuristic. `solution_record` /
  `solution_update` stamp the active conversation id on each new revision.
  Three read-only tools surface the layer:
  - **`conversation_list`** — recent conversations, most-recently-active first.
  - **`conversation_show { id }`** — every tool call in one conversation
    (chronological), plus the solutions whose revisions came from it. Answers
    "what else happened in this conversation?"
  - **`solution_conversations { id }`** — the conversation(s) that contributed
    revisions to a recorded solution. Answers "what conversation was this a
    part of?" Many-to-many via the revisions table.
  - `solution_show` now also displays the `conversation_id` per revision.
  Migration `0002_conversations` adds the new tables + a nullable
  `solution_revisions.conversation_id` column; legacy revisions remain NULL.

### Changed

- **`[memory].enabled` now defaults to `true`.** The persistent memory layer is
  local (SQLite under `[memory].dir`, no network), so there's no privacy or
  security cost to having it on; the value it adds — intrinsic recall surfacing
  prior solutions as a preamble on every query-bearing tool call — is what
  gives the model an "I solved this before" surface that's hard to get any
  other way. Set `enabled = false` to silence the family entirely.

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

- **Persistent memory & solution-history skills** (on by default, `[memory]`).
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
- **Space, markets & science skills** (keyless): `nasa_neo` / `nasa_mars_photos`
  (api.nasa.gov, `DEMO_KEY` by default, optional `[nasa].key`);
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
