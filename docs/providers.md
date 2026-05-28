# Providers reference

A **provider** is one source of results behind a search tool. Every provider
implements the `SearchProvider` trait (`id`, `kind`, async `search`) and is
selected and ordered per kind in configuration. This page documents each
provider in detail; for the *architecture* (the trait, the spec-driven families,
how to add one) see [CONTRIBUTING.md](../CONTRIBUTING.md).

## How providers combine

- **Kinds.** Each provider serves one kind: `web` (general web search), `code`
  (source-code search), or `qa` (question/answer sites). The tools map to kinds:
  `web_search` → web, `code_search` → code, `stackexchange_search` → qa.
- **Selection & order.** `config/02-search.toml` (`[providers].web/code/qa`)
  lists the providers per kind, in priority order. Unknown ids are skipped with a
  warning.
- **Strategy** (`[search].strategy`): `fallback` (first provider with results
  wins) or `aggregate` (query all concurrently, dedupe by URL, re-rank).
- **Ranking** (`[search].ranking`, aggregate only): `reciprocal` (default),
  `borda`, `breadth` (consensus), or `interleave` (round-robin).
- **Render** (golden rule: scrape is default). HTML-scraping providers fetch over
  plain HTTP unless the model sets `render=true`, which routes them through the
  shared headless browser. The `google` engine is the one always-render provider.
- **Keyless** (golden rule). Everything works without accounts/keys; the only
  credentials are *optional* (a GitHub token, a StackExchange key).

Config lives in granular per-provider files under `config/providers/`; see
[`config.example`-style files](../config/) and the README for the full schema.

---

## Web search providers

### `duckduckgo`
- **Kinds:** web, code · **Keyless:** yes · **Family:** engine (`EngineSpec`)
- **How:** Scrapes the `lite.duckduckgo.com` HTML endpoint (POST). Honors the
  `site:` operator, so in code mode it scopes precisely to `[code].sites`.
- **Render:** honored (`render=true` loads the same query in the browser, which
  can slip past rate-limiting).
- **Caveats:** DuckDuckGo rate-limits aggressively by IP (especially datacenter
  IPs); pair it with `mojeek` as a fallback.

### `mojeek`
- **Kinds:** web, code · **Keyless:** yes · **Family:** engine
- **How:** Scrapes `www.mojeek.com/search` (GET) — an independent index that is
  far more tolerant of automation, making it a reliable fallback. It has no
  `site:` operator, so code mode appends the forge domains as keywords and filters
  results to them.
- **Render:** honored.

### `google`
- **Kinds:** web, code · **Keyless:** yes (no API key) · **Family:** engine
- **How:** Drives the shared **headless Chrome** (`Method::Browser`) to load
  `google.com/search`, with a custom parser for Google's markup. Honors `site:`
  for code mode.
- **Requirements:** a local Chrome/Chromium at runtime (the browser is always
  compiled in; Chrome is only needed when this runs).
- **Caveats:** the one **always-render** provider (Google has no scrapeable
  endpoint). CAPTCHA-prone on datacenter IPs; keep `mojeek` in the chain. Not in
  the default lists — add `"google"` to opt in.

### `medium`
- **Kinds:** web · **Keyless:** yes · **Family:** bespoke (RSS)
- **How:** Medium's search is bot-walled, so this treats the query as a Medium
  **tag** and returns recent articles from `medium.com/feed/tag/<tag>` (RSS).
- **Caveats:** it surfaces *recent posts for a topic*, not full-text relevance
  search. Not in the default lists — add `"medium"` to opt in.

---

## Code search providers

### `github`
- **Kind:** code · **Keyless:** yes (token optional) · **Family:** composite
- **How:** Two modes chosen at runtime:
  - **default (keyless):** a site-scoped web scrape of `github.com` (reuses the
    shared `forge::search` — DuckDuckGo → Mojeek, render-aware).
  - **token set:** GitHub's authenticated code-search API
    (`api.github.com/search/code`), which returns matched code fragments as
    snippets. GitHub no longer allows *unauthenticated* API code search, so the
    API path needs a token; the scrape path never does.
- **Config:** `config/providers/github.toml` → `[github].token` (or
  `GITHUB_TOKEN` / `LODESTONE_GITHUB_TOKEN`). A classic PAT with `public_repo`,
  or a fine-grained PAT with read-only Contents.

### `gitlab` · `codeberg` · `gitea`
- **Kind:** code · **Keyless:** yes · **Family:** forge (`ForgeSpec`)
- **How:** Each is a site-scoped web search of its domain (`gitlab.com`,
  `codeberg.org`, `gitea.com`) via the shared `forge::search`, parsing that
  forge's blob-URL layout into `(repo, path)`. Render-aware.
- **Caveats:** results depend on the search engines indexing those forges (often
  thinner than GitHub). Not in the default lists — add them to opt in. Reading a
  result file works via `fetch_repo_file` (handles GitLab/Gitea blob URLs).

### `grep_app`
- **Kind:** code · **Keyless:** yes · **Family:** bespoke (JSON API)
- **How:** Queries grep.app's JSON code-search endpoint and builds GitHub blob
  URLs from the hits.
- **Caveats:** grep.app is frequently behind a bot-challenge that returns HTML
  instead of JSON; when that happens it yields nothing and the chain falls
  through.

### `duckduckgo` / `mojeek` / `google` (code mode)
The web engines above also serve `code` kind: they run a `site:`-scoped (or
keyword-scoped, for Mojeek) search over `[code].sites` and parse GitHub/GitLab/
Gitea result URLs into `(repo, path)`. This is the keyless default for GitHub
code search when no `github` token is set.

---

## Q&A providers

### `stackoverflow` (alias `stackexchange`)
- **Kind:** qa · **Keyless:** yes (key optional) · **Family:** composite
- **How:** Two modes:
  - **default:** the keyless StackExchange public API
    (`api.stackexchange.com`). An optional key raises the per-IP quota.
  - **`render=true`:** scrapes `stackoverflow.com/search` via the headless
    browser (no quota); applies to the `stackoverflow` site only.
- **Config:** `config/providers/stackexchange.toml` →
  - `default_site` — the site slug used when a call doesn't specify one
    (`stackoverflow`, `serverfault`, `superuser`, `askubuntu`, `unix`, …; the
    `api_site_parameter` from <https://api.stackexchange.com/2.3/sites>).
  - `key` — optional API key (raises quota; not a login). Prefer
    `LODESTONE_STACKEXCHANGE_KEY`.
  - `allowed_sites` — guardrail allowlist of site slugs (empty = any).
- **Related tool:** `stackexchange_answers` reads a question's body and top
  answers (always via the API; honors the same `default_site`/key/allowlist).
