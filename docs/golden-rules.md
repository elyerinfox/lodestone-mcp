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
   [`docs/providers/`](providers/), an index row in
   [`docs/providers.md`](providers.md), and a README provider-table row (see the
   contribution checklist in [CONTRIBUTING.md](../CONTRIBUTING.md)).
