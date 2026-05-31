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
