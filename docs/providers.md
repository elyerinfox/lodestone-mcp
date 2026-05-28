# Providers reference

A **provider** is one source of results behind a search tool. Every provider
implements the `SearchProvider` trait (`id`, `kind`, async `search`) and is
selected and ordered per kind in configuration. This page is the **index**; each
provider has its own detailed page under [`docs/providers/`](providers/) covering
its rationale, features, skills (tools), config, and schema/structs. For the
*architecture* (the trait, the spec-driven families, how to add one) see
[CONTRIBUTING.md](../CONTRIBUTING.md).

## How providers combine

- **Kinds.** Each provider serves one kind: `web` (general web search), `code`
  (source-code search), or `qa` (question/answer sites). The tools map to kinds:
  `web_search` → web, `code_search` → code, `qa_search` → qa.
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

### Tools per provider

Each provider participates in its kind's **general** search tool (`web_search` /
`code_search` / `qa_search`) and also gets a **per-provider** tool named
`<kind>_<id>` (e.g. `web_mojeek`, `code_github`, `qa_stackoverflow`) to target it
alone. StackOverflow additionally exposes the bespoke `qa_stackoverflow_answers`
skill. All tools are gateable via `[tools]`.

Config lives in granular per-provider files under `config/providers/`; each
provider page documents its own properties. Providers are grouped below by
**family** (matching `src/providers/`), not by kind.

---

## Engine family — spec-driven search (`src/providers/engine/`)

Shared `HtmlEngineProvider` driven by an `EngineSpec`. Each engine serves **both
`web` and `code`** kinds (code mode scopes to the forges in `[code].sites`).
`render` is honored per call.

| Provider | Default | Notes |
| --- | --- | --- |
| [`duckduckgo`](providers/duckduckgo.md) | on (web+code) | Keyless `lite.duckduckgo.com`; honors `site:`. Rate-limits by IP. |
| [`mojeek`](providers/mojeek.md) | on (web+code) | Keyless independent index; tolerant fallback. Keyword-scoped code. |
| [`google`](providers/google.md) | off | Always-render (headless Chrome); broadest index, CAPTCHA-prone. |

## Forge family — spec-driven code search (`src/providers/forge/`)

Shared `ForgeCodeProvider` / `forge::search` driven by a `ForgeSpec`: a keyless,
site-scoped web search of one forge (DuckDuckGo → Mojeek, render-aware) with that
forge's blob-URL layout parsed into `(repo, path)`. Kind: **code**.

| Provider | Default | Notes |
| --- | --- | --- |
| [`gitlab`](providers/gitlab.md) | off | Site-scoped search of `gitlab.com`. |
| [`codeberg`](providers/codeberg.md) | off | Site-scoped search of `codeberg.org` (Gitea). |
| [`gitea`](providers/gitea.md) | off | Site-scoped search of `gitea.com`. |

## Composite providers — multi-mode (`src/providers/composite/`)

Bespoke shells that pick a sourcing mode at runtime, reusing a family for one of
them.

| Provider | Kind | Default | Notes |
| --- | --- | --- | --- |
| [`github`](providers/github.md) | code | off | Keyless scrape by default; GitHub code-search API with an optional token. |
| [`stackoverflow`](providers/stackexchange.md) | qa | on | Keyless StackExchange API (optional key); `render=true` scrapes SO. Adds `qa_stackoverflow_answers`. |

## Bespoke providers — unique transport/parse (`src/providers/bespoke/`)

| Provider | Kind | Default | Notes |
| --- | --- | --- | --- |
| [`grep_app`](providers/grep_app.md) | code | on | grep.app JSON code-search; true substring matches. Falls through if bot-walled. |
| [`medium`](providers/medium.md) | web | off | Per-tag RSS feed; recent posts for a topic (not full-text search). |
| [`searxng`](providers/searxng.md) | web/code | off | Self-hosted SearXNG metasearch JSON API. Needs `[searxng].url`. |
