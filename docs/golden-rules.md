# Golden rules (non-negotiable)

These are the project's invariants. New code and providers must uphold them; a
change that breaks one is wrong by definition. This file is the single source of
truth — the README and CONTRIBUTING link here rather than restating them.

1. **Scrape is the default; render is optional and a fallback.** Every source
   fetches over plain HTTP by default. The headless browser is never the default
   path — it runs only when the model explicitly asks for it (a `render` flag on a
   search, or the dedicated `render_page` tool), as a fallback when a plain fetch
   isn't enough. The server never silently substitutes rendering. (The sole
   exception is the `google` engine, which has no scrapeable endpoint and is
   therefore browser-only and strictly opt-in via config.)

2. **The LLM always decides.** Rendering is a per-call `render` flag the calling
   model sets; the server never enables it on its own. The model likewise drives
   what to retrieve next. We expose capabilities and defaults — we don't make the
   call for it.

3. **Keyless by default.** No source requires an account or key on the default
   path. Credentials (a GitHub token, a StackExchange key, the keyed `brave` /
   `google_cse` web engines) are strictly optional enhancements layered over the
   keyless providers, never a precondition — a keyed provider is off until its key
   is set and never replaces a keyless one.

4. **Parallelize — always.** Independent work must run concurrently, never
   sequentially. Aggregate search sources every provider on its own task across
   the multi-threaded runtime; any new multi-source or I/O-bound path must overlap
   its work (`tokio::spawn` / `join`) and must never block the runtime with sync
   I/O or long CPU work on the async threads.

5. **Everything is enable/disable-able.** Every capability ships with an explicit
   off switch. Tools are gated by `[tools]` (allow/deny). Each provider is gated by
   membership in its `[providers].<kind>` list and by its per-provider tool. Any
   new subsystem must add its own flag (e.g. `[cache].enabled`, `[network].enabled`,
   `[network].mdns`) — no capability is always-on-only.

6. **Every provider is documented.** A provider is not done until an end user can
   understand and enable it without reading the source: a per-provider page under
   [`docs/providers/`](providers/) (a shared family page is fine for spec-driven
   families) and an index row in [`docs/providers.md`](providers.md). The README
   stays a concise overview and links out to that reference (see the contribution
   checklist in [CONTRIBUTING.md](../CONTRIBUTING.md)).

7. **Every tool is a self-contained skill module under a common contract.** No
   tool/skill logic lives in `main.rs` — it is bootstrap and wiring only. Each tool
   is its own module under [`src/skills/`](../src/skills/) implementing the shared
   `Skill` contract (`name` / `description` / `schema` / `call`), and is assembled
   into the router from that registry. A skill's own domain logic (its API/socket
   client, parsers, formatters — e.g. the Docker, Kubernetes, OCI, translate
   clients) lives *with the skill* under `src/skills/`, never as a loose module at
   the `src/` root. Data-source `SearchProvider`s remain under
   [`src/providers/`](../src/providers/); skills may build on them. The paradigm is
   uniform: adding a capability means adding a skill module, not editing `main.rs`.

8. **Destructive actions never fire unguarded.** Any tool that deletes, removes,
   overwrites, or otherwise makes a hard-to-reverse change must do **exactly one** of
   the following before it acts — never just run:
   1. **Prompt the user** for confirmation (e.g. MCP elicitation where the client
      supports it), or
   2. **Be disabled** — gated off by config so it isn't exposed at all (golden rule
      5; e.g. `[filesystem].enabled` off by default), or
   3. **Go through a guard challenge** — route through the confirmation
      [`guard`](../src/skills/guard.rs): the first call performs nothing and returns a
      one-time `confirm` token; a second call with that token executes (and
      `trust=true` whitelists the action for the session).

   The guard (option 3) is the default and is client-agnostic by design — it must
   **not** depend on MCP elicitation, since some clients (e.g. LM Studio) don't
   support it. A family's `allow_destructive` flag pre-authorizes the action (skips
   the prompt) but is never required for the tool to exist. Destructive tools are
   exposed and gated at *call time*, not silently hidden.

9. **One tool per method — no hidden auto-selection.** Each distinct
   methodology, algorithm, or mode is its own explicitly-named tool. A tool must
   **not** silently fork between genuinely different methods based on an optional
   argument or an internal heuristic — that hides the choice from the caller and
   takes the decision away from the model (golden rule 2). Be granular: prefer
   `forecast_holt_linear` + `forecast_holt_winters` over one `forecast` that guesses
   from a `season_length`; prefer `hf_model_search` + `hf_dataset_search` over one
   `hf_search` with a `kind` flag. The method belongs **in the tool name**, so the
   model picks it by picking the tool and the schema/description can be specific to
   that one method.

   This is about *distinct methods*, not *targets* or *parameters*. It is **fine**
   to:
   - select by an explicit required id where the id *is* the method — a
     named-formula registry (`physics_formula { name: "kinetic_energy" }`), a
     resource `kind` in a polymorphic API (`k8s_get`), or a search `strategy` the
     user sets deliberately;
   - address different *targets* through one interface when the user names the
     target — `db_query` runs SQL against whatever the connection URL points to
     (Postgres/MySQL); the scheme is the user's explicit choice, not a hidden
     algorithm. (A genuinely different protocol still gets its own tool, e.g.
     `redis_command`.)

   The test: if a caller would be surprised *which* method ran, split it. If the
   caller named it (by id, target, or required enum), it's already explicit.

10. **`cargo fmt` and `cargo clippy --all-targets -- -D warnings` before every
    commit.** Both must pass; CI enforces both. Formatting is non-negotiable so
    diffs stay about meaning rather than whitespace, and clippy at
    deny-on-warning is non-negotiable so subtle correctness, performance, and
    idiom regressions (uninitialized locks, needless clones, panicking
    `unwrap`s in async paths, …) don't accumulate. Running them locally before
    `git commit` is faster than waiting for CI to reject the push. See the
    "Build & verify" section in [CONTRIBUTING.md](../CONTRIBUTING.md) for the
    exact commands, what each flag means, and editor / pre-commit-hook
    integration tips. Never `--no-verify` past a failing clippy or fmt check;
    fix the underlying issue.

11. **Sensitive information must never be shared.** Credentials (API keys,
    tokens, bearer secrets, database connection strings, passwords), personally
    identifying information the server happens to see, and any other secret
    material **must never** be:
    - **logged** — not at any level, not even debug. `tracing` calls that
      reference a config field carrying a secret must redact it (`<set>` /
      `<unset>`, not the value).
    - **returned in a tool response** — the [`features`](skills/meta.md) tool
      is the load-bearing example: it surfaces `[github].token`, `[eia].key`,
      `[network].token`, the DATABASE_URL passed to `db_query`, etc. as the
      tokens `<set>` / `<unset — …>` and never the underlying value. Every
      future tool that introspects config follows the same pattern.
    - **committed to git** — `lodestone.toml` (the personal-overrides file)
      is gitignored; the shipped `config/` baseline must contain no real
      credentials. Database URLs passed to `db_query` / `redis_command` are
      conversation-supplied at call time, never stored on disk.
    - **advertised over the constellation** — the digest carries only
      hashes of normalized query keys; the cached results that traverse a
      consult call never include payloads from keyed sources (golden rule 3
      keeps the keyless path the default for sharing).
    - **echoed back when the model pastes one in** — input that *looks* like
      a secret (`sk-…`, `ghp_…`, JWT shape, `*://user:pass@host` URI form, …)
      must be redacted before reaching any response, the same as if the
      server had seen it through config.

    `ct_eq` ([`util.rs`](../src/util.rs)) is the constant-time comparator for
    any bearer-token check — never use `==` on secret bytes (timing leak).
    When you add a tool that takes a secret-shaped argument (a database URL,
    an API key, a webhook URL), document explicitly that it's a secret in
    its arg's `#[doc]` and verify it doesn't escape into any response or
    log line.

12. **Cite your sources; make outputs auditable and verifiable.** Every
    skill (and every solution / formula / vendored data table within a
    skill) that makes a factual or mathematical claim must:

    1. **Cite the canonical source** for each formula, constant, and
       vendored data table. Cite in the module's top-doc *and* in the
       relevant tool's `description()` (which the LLM reads at
       `tools/list` time). Cite primary literature when it exists
       (Krane *Introductory Nuclear Physics* §3.3; ICRP Publication 103
       Annex B; AME2020; NIST CODATA 2022; ITU-R P.838-3; …), or a
       canonical engineering reference (Machinery's Handbook 31st ed.;
       Shigley's *Mechanical Engineering Design* Table A-9; RFC 9106;
       …). Generic textbook attribution ("standard formula") is **not** a
       citation — name the source.
    2. **Pin the version / edition** of any cited reference, table, or
       data file. "Atomic weights" without "IUPAC CIAAW 2021" hides
       which revision was vendored; "OWASP recommendation" without
       "OWASP Password Storage Cheat Sheet 2023" goes stale silently.
    3. **Surface intentional limits explicitly.** If a formula is a
       *simplified* form of a canonical model (Saastamoinen without the
       `B(h)`/`δR` terms; ITU-R P.838 without the full 4-term fit), the
       tool description must say so in one short sentence. Same for
       narrow-beam attenuation, idealized point sources, small-angle
       approximations, etc. The model needs to know when an answer is a
       first-cut estimate.
    4. **Be auditably verifiable.** Each skill must ship at least one
       unit test that reproduces a known-correct result from the cited
       source — a textbook worked example, a tabulated value, or a
       canonical test vector (D + T → ⁴He + n: Q = 17.589 MeV; CRC-32
       of `"123456789"` = 0xCBF43926; IUPAC abridged atomic weight of
       Fe = 55.845 g/mol). For purely numerical tools the test is the
       golden value with a documented tolerance; for symbolic /
       structural tools (e.g. a G-code emitter, a CoT encoder) the
       test confirms the wire-format conformance the standard
       requires.
    5. **Validation goes in `docs/audit-report.md`.** The audit report
       is the project's running ledger of how each factual claim was
       verified and where the source lives. When you change a formula
       or refresh a vendored table, update the audit-report entry and
       cite the change.

    Why: the LLM uses these tools to ground its answers. A tool that
    silently rounds, picks the wrong table, or quotes an out-of-date
    constant doesn't just produce wrong output — it produces *plausibly
    wrong output* the model will defend. Citation + auditable verification
    is what keeps every layer above the tool honest. The 0.1.6 cross-
    codebase audit ([`audit-report.md`](audit-report.md)) caught eight
    wrong-answer bugs in tools that had been merged because they "looked
    right"; this rule exists so future tools never reach that state.

13. **Remotely-retrieved academic and scientific data is shared over the
    constellation.** Helping the greater good is part of the project's
    purpose: a paper or dataset one node fetched should be reachable
    from any other lodestone in the same constellation without re-hitting
    the upstream. Every tool that pulls academic / scientific content
    from a remote source — papers (arXiv, PubMed, PMC, OpenAlex,
    Unpaywall, doi.org), encyclopedic references (Wikipedia, RFC,
    standards), life-sciences entries (UniProt, RCSB PDB, Ensembl, NCBI
    databases), open-science feeds (NASA, NOAA, USGS, ESA, SWPC, IAEA,
    OpenSky) — **must** route through `retrieval_get` / `retrieval_put`
    so the response is keyed by a stable canonical identifier and joins
    the constellation digest on the next sync.

    Stable canonical keys are non-negotiable: a peer must be able to
    ask for the same artifact by the same key independently. Examples
    of the canonical-key shape the existing tools already use:

    - `arxiv|<id>`, `arxiv_search|<query>`
    - `pubmed|<pmid>`, `pubmed_search|<query>`, `ncbi|<db>|<id>`
    - `unpaywall|<doi>`, `openalex|<id>`
    - `wikipedia|<lang>|<title>`
    - `rfc|<number>`, `standards|<query>`
    - `uniprot|<accession>`, `pdb|<id>`, `ensembl|<id>|expand=<bool>`
    - `nasa_neo|<date>`, `nasa_mars|<rover>|<sol>|<limit>`
    - `swpc|planetary-k-index`, `swpc|plasma-1-day`
    - `usgs_quake|<minimum>|<period>`, `opensky|<bbox>`

    When you add the next academic / scientific retrieval tool:

    1. **Pick the canonical key.** Use the upstream's stable identifier
       (DOI, accession, PMID, etc.). If there are multiple aliases for
       the same artifact (raw URL + DOI + arxiv id), use
       [`retrieval_put_indexed`](../src/main.rs) so the entry is
       discoverable under every alias.
    2. **Look up by key before the network call.** `if let Some(c) =
       server.retrieval_get(&key).await { return Ok(text_result(c)); }`
       — this consults the local TTL cache and, on miss, asks every
       Bloom-matching peer before falling through.
    3. **Write the canonical body back** after a successful fetch.
       `server.retrieval_put(key, &body);` advertises the new entry on
       the next constellation digest cycle.
    4. **Keep keyed-source payloads out of the cache.** Golden rule 11
       still applies — a record fetched from a keyed provider (a paid
       endpoint, a token-gated search) **must not** be `retrieval_put`
       since the body would then be served to peers without the
       consenting credential. Use the request-scoped cache instead.
    5. **Document the share** in the per-skill doc's *Constellation
       sharing* section (see [skills/bio_data.md](skills/bio_data.md) for
       the reference shape) so the operator knows the artifact crosses
       the mesh.

    The constellation already enforces the privacy and trust model
    around what crosses the wire: only hashes of the canonical keys
    appear in the digest (raw queries never traverse), the multi-peer
    consensus floor (`[network].min_agreement`) bounds single-peer
    influence, and the on-the-wire payloads are exactly what the
    skill chose to `retrieval_put` — no implicit fan-out of anything
    else. The rule above is about *what tools route through that
    mechanism*. Academic and scientific retrieval should; per-user or
    locally-computed work shouldn't.

    The audit in [audit-report.md](audit-report.md) tracks which
    tools currently comply and which are queued for retrofit.
