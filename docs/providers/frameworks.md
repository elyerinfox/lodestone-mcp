# Framework documentation (docsite family)

|  |  |
| --- | --- |
| **Family** | docsite (`DocSiteProvider` + `DocSiteSpec`) |
| **Kind** | `docs` |
| **Default-on** | `php`, `laravel`, `vue`, `react`, `svelte` — the rest are opt-in |
| **Keyless** | yes |
| **Render** | honored per call (passed through to the underlying engine search) |
| **Code** | [`src/providers/docsite/mod.rs`](../../src/providers/docsite/mod.rs) |
| **Config** | [`config/02-search.toml`](../../config/02-search.toml) (`[providers].docs`) · custom hosts: [`config/07-docsites.toml`](../../config/07-docsites.toml) |

## Why
Package registries (crates.io, npm, …) expose uniform keyless JSON search APIs, so
the [registry family](../providers.md#registry-family--spec-driven-docpackage-search-srcprovidersregistry)
covers them cleanly. **Framework documentation sites don't** — PHP, Laravel, Vue,
React, Svelte and friends each have bespoke (often Algolia-keyed, JS-rendered)
search. Rather than wire up a key per framework (violating the keyless golden
rule), each framework is a **site-scoped web search** — exactly the pattern the
[forge family](../providers.md#forge-family--spec-driven-code-search-srcprovidersforge)
uses for code. A provider differs only in its domain, so the built-ins are a small
static table and you can register more with one line.

## How it works
`docsite::search` runs the query as `site:<domain> <query>` through the shared
engine path (scrape-first: DuckDuckGo, then Mojeek as a keyword-scoped fallback),
filters results to the framework's domain, and returns them. It carries no
framework-specific parsing — doc pages are read as-is.

Read a result with [`fetch_page`](../../src/main.rs) (plain HTTP). Many modern doc
sites are JS-heavy SPAs; if a plain fetch comes back thin, use
[`render_page`](../../src/main.rs) or pass `render=true` to the search to route the
underlying fetch through the headless browser.

## Built-in frameworks
| id | domain | default |
| --- | --- | --- |
| `php` | `php.net` | on |
| `laravel` | `laravel.com` | on |
| `vue` | `vuejs.org` | on |
| `react` | `react.dev` | on |
| `svelte` | `svelte.dev` | on |
| `angular` | `angular.dev` | off |
| `nextjs` | `nextjs.org` | off |
| `nuxt` | `nuxt.com` | off |
| `django` | `docs.djangoproject.com` | off |
| `flask` | `flask.palletsprojects.com` | off |
| `fastapi` | `fastapi.tiangolo.com` | off |
| `rails` | `guides.rubyonrails.org` | off |
| `spring` | `docs.spring.io` | off |
| `tailwind` | `tailwindcss.com` | off |
| `express` | `expressjs.com` | off |
| `symfony` | `symfony.com` | off |
| `astro` | `docs.astro.build` | off |
| `solid` | `docs.solidjs.com` | off |
| `docker` | `docs.docker.com` | on |
| `kubernetes` | `kubernetes.io` | on |
| `helm` | `helm.sh` | on |
| `ieee` | `ieeexplore.ieee.org` | on |
| `sae` | `sae.org` | on |
| `nist` | `nist.gov` | on |
| `kernel` | `docs.kernel.org` | on |
| `ffmpeg` | `ffmpeg.org` | on |
| `nvidia` | `docs.nvidia.com` | on |
| `intel_arc` | `intel.com` | on |

Enable more by adding their ids to `[providers].docs` in `config/02-search.toml`.

> For **container/cloud-native data** (not just docs) — Docker Hub image search,
> tags and metadata, OCI-registry inspection, and Artifact Hub (Helm/Operators) —
> see the dedicated tools in [docs/containers.md](../containers.md).

## Custom doc sites
Register any documentation host with a `[docsites.<id>]` table (see
[`config/07-docsites.toml`](../../config/07-docsites.toml)):

```toml
[docsites.mydocs]
domain = "docs.example.com"
```

Then add `"mydocs"` to `[providers].docs`. It behaves exactly like a built-in and
gets a `docs_mydocs` tool.

## Caveats
- Result quality depends on the search engines having indexed the doc site.
- Each enabled framework adds a DuckDuckGo request to an aggregated `docs_search`.
  The five defaults are a deliberate balance; trim the list if you don't need them
  or hit rate limits.

## Skills (tools)
- **General:** each enabled framework participates in `docs_search` (aggregated and
  re-ranked across all configured `docs` providers, registries included).
- **Per-provider:** `docs_<id>` (e.g. `docs_react`, `docs_php`, `docs_laravel`) to
  target one framework. Args: `query`, `max_results?`, `language?`, `render?`
  ([`ProviderSearchArgs`](../../src/main.rs)).

## Schema / structs
Declares a [`DocSiteSpec`](../../src/providers/docsite/mod.rs):

```rust
DocSiteSpec {
    id: "react",
    domain: "react.dev",
}
```

Shared types: `SearchProvider`, `SearchQuery`, `SearchResult`
([`src/provider.rs`](../../src/provider.rs)).
