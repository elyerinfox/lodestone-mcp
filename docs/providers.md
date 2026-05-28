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

Config lives in granular per-provider files under `config/providers/`; see those
files and the README for the full schema. Providers below are grouped by
**family** (matching `src/providers/`), not by kind.

---

## Engine family — spec-driven search (`src/providers/engine/`)

Shared `HtmlEngineProvider` driven by an `EngineSpec`. Each engine serves **both
`web` and `code`** kinds; in code mode it scopes to the forges in `[code].sites`.
`render` is honored per call (the model's opt-in).

### `duckduckgo`
- **Keyless:** yes. Scrapes `lite.duckduckgo.com` (POST). Honors `site:`, so code
  mode scopes precisely.
- **Caveats:** rate-limits aggressively by IP (esp. datacenter IPs); pair with
  `mojeek`. `render=true` can slip past the rate-limit.

### `mojeek`
- **Keyless:** yes. Scrapes `www.mojeek.com/search` (GET); an independent index,
  very tolerant of automation — the reliable fallback. No `site:`, so code mode
  appends the forge domains as keywords and filters results to them.

### `google`
- **Keyless:** yes (no API key). Drives the shared **headless Chrome**
  (`Method::Browser`) over `google.com/search` with a custom parser; honors
  `site:` for code mode.
- **Requirements / caveats:** the one **always-render** provider (Google has no
  scrapeable endpoint); needs a local Chrome at runtime, CAPTCHA-prone on
  datacenter IPs. Not in the default lists — add `"google"` to opt in.

---

## Forge family — spec-driven code search (`src/providers/forge/`)

Shared `ForgeCodeProvider` / `forge::search` driven by a `ForgeSpec`: a keyless,
site-scoped web search of one forge (DuckDuckGo → Mojeek, render-aware) with that
forge's blob-URL layout parsed into `(repo, path)`. Kind: **code**.

### `gitlab` · `codeberg` · `gitea`
- **Keyless:** yes. Search `gitlab.com`, `codeberg.org`, and `gitea.com`
  respectively.
- **Caveats:** results depend on the search engines indexing those forges (often
  thinner than GitHub). Not in the default lists — add them to opt in. Read a
  result file with `fetch_repo_file` (handles GitLab/Gitea blob URLs).

---

## Composite providers — multi-mode (`src/providers/composite/`)

Bespoke shells that pick a mode at runtime, reusing a family for one of them.

### `github` (kind: code)
- **Keyless:** yes (token optional). Two modes:
  - **default (keyless):** site-scoped web scrape of `github.com`, reusing
    `forge::search` (DuckDuckGo → Mojeek, render-aware).
  - **token set:** GitHub's authenticated code-search API
    (`api.github.com/search/code`), returning matched code fragments as snippets.
    GitHub dropped *unauthenticated* API code search, so the API path needs a
    token; the scrape path never does.
- **Config:** `config/providers/github.toml` → `[github].token` (or
  `GITHUB_TOKEN` / `LODESTONE_GITHUB_TOKEN`).

### `stackoverflow` (alias `stackexchange`, kind: qa)
- **Keyless:** yes (key optional). Two modes:
  - **default:** the keyless StackExchange API (`api.stackexchange.com`); an
    optional key raises the per-IP quota.
  - **`render=true`:** scrapes `stackoverflow.com/search` via the headless
    browser (no quota); `stackoverflow` site only.
- **Config:** `config/providers/stackexchange.toml` →
  - `default_site` — site slug used when a call omits one (`stackoverflow`,
    `serverfault`, `superuser`, `askubuntu`, `unix`, …; the `api_site_parameter`
    from <https://api.stackexchange.com/2.3/sites>).
  - `key` — optional API key (raises quota; not a login). Prefer
    `LODESTONE_STACKEXCHANGE_KEY`.
  - `allowed_sites` — guardrail allowlist of site slugs (empty = any).
- **Related tool:** `stackexchange_answers` reads a question's body + top answers
  (always via the API; honors the same `default_site`/key/allowlist).

---

## Bespoke providers — unique transport/parse (`src/providers/bespoke/`)

### `grep_app` (kind: code)
- **Keyless:** yes. Queries grep.app's JSON code-search endpoint and builds GitHub
  blob URLs from the hits.
- **Caveats:** frequently behind a bot-challenge that returns HTML instead of
  JSON; when that happens it yields nothing and the chain falls through.

### `medium` (kind: web)
- **Keyless:** yes. Medium's search is bot-walled, so this treats the query as a
  Medium **tag** and returns recent articles from `medium.com/feed/tag/<tag>`
  (RSS).
- **Caveats:** surfaces *recent posts for a topic*, not full-text relevance
  search. Not in the default lists — add `"medium"` to opt in.
