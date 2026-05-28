//! Framework-documentation providers (`docs` kind).
//!
//! Modern framework docs (PHP, Laravel, Vue, React, Svelte, …) don't expose a
//! uniform keyless JSON search API the way package registries do, so — exactly
//! like the `forge` family does for code — each one is a **site-scoped web
//! search**: scrape-first (DuckDuckGo, then Mojeek), filtered to the framework's
//! documentation domain. A provider differs only in its [`DocSiteSpec`] (id +
//! domain), so the built-ins are a small static table and users can register more
//! via `[docsites.<id>]`.
//!
//! Golden rules: keyless and plain-HTTP by default; `render` is honored per call
//! (these doc sites are often JS-heavy SPAs, so the model can set `render=true`
//! when a plain fetch comes back thin). Each is independently enable/disable-able
//! via `[providers].docs` and its auto-generated `docs_<id>` tool.

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use super::engine;
use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};

/// Declarative description of a documentation site: an id and the host the search
/// is scoped to. (No URL parsing — unlike forges, doc pages are read as-is.)
pub(super) struct DocSiteSpec {
    pub id: &'static str,
    pub domain: &'static str,
}

/// Built-in framework documentation sites. The default-enabled set (see
/// `config/02-search.toml`) is the five most-requested; the rest are opt-in by
/// adding their id to `[providers].docs`.
static SITES: &[DocSiteSpec] = &[
    DocSiteSpec {
        id: "php",
        domain: "php.net",
    },
    DocSiteSpec {
        id: "laravel",
        domain: "laravel.com",
    },
    DocSiteSpec {
        id: "vue",
        domain: "vuejs.org",
    },
    DocSiteSpec {
        id: "react",
        domain: "react.dev",
    },
    DocSiteSpec {
        id: "svelte",
        domain: "svelte.dev",
    },
    DocSiteSpec {
        id: "angular",
        domain: "angular.dev",
    },
    DocSiteSpec {
        id: "nextjs",
        domain: "nextjs.org",
    },
    DocSiteSpec {
        id: "nuxt",
        domain: "nuxt.com",
    },
    DocSiteSpec {
        id: "django",
        domain: "docs.djangoproject.com",
    },
    DocSiteSpec {
        id: "flask",
        domain: "flask.palletsprojects.com",
    },
    DocSiteSpec {
        id: "fastapi",
        domain: "fastapi.tiangolo.com",
    },
    DocSiteSpec {
        id: "rails",
        domain: "guides.rubyonrails.org",
    },
    DocSiteSpec {
        id: "spring",
        domain: "docs.spring.io",
    },
    DocSiteSpec {
        id: "tailwind",
        domain: "tailwindcss.com",
    },
    DocSiteSpec {
        id: "express",
        domain: "expressjs.com",
    },
    DocSiteSpec {
        id: "symfony",
        domain: "symfony.com",
    },
    DocSiteSpec {
        id: "astro",
        domain: "docs.astro.build",
    },
    DocSiteSpec {
        id: "solid",
        domain: "docs.solidjs.com",
    },
    // Cloud-native / tooling docs.
    DocSiteSpec {
        id: "docker",
        domain: "docs.docker.com",
    },
    DocSiteSpec {
        id: "kubernetes",
        domain: "kubernetes.io",
    },
    DocSiteSpec {
        id: "helm",
        domain: "helm.sh",
    },
];

/// Construct a built-in doc-site provider by id, if known.
pub(super) fn make(id: &str) -> Option<DocSiteProvider> {
    SITES
        .iter()
        .find(|s| s.id == id)
        .map(|spec| DocSiteProvider { spec })
}

/// Build a provider for a user-configured documentation site (`[docsites.<id>]`).
/// The id/domain/spec are leaked to `'static` (config doc-sites live for the whole
/// process), reusing the shared site-scoped search.
pub(super) fn make_configured(id: &str, domain: &str) -> Option<DocSiteProvider> {
    if id.is_empty() || domain.is_empty() {
        tracing::warn!(id, "configured docsite missing a domain; skipping");
        return None;
    }
    let spec: &'static DocSiteSpec = Box::leak(Box::new(DocSiteSpec {
        id: Box::leak(id.to_string().into_boxed_str()),
        domain: Box::leak(domain.to_string().into_boxed_str()),
    }));
    Some(DocSiteProvider { spec })
}

/// Shared site-scoped documentation search for one framework. Scrape-first
/// (DuckDuckGo, then Mojeek); `render` is honored only when the caller asked.
async fn search(
    spec: &DocSiteSpec,
    http: &Client,
    query: &SearchQuery,
) -> Result<Vec<SearchResult>> {
    let terms = &query.text;

    // DuckDuckGo honors `site:`; fall back to Mojeek (keyword-scoped) on empty.
    let ddg_query = format!("site:{} {terms}", spec.domain);
    let mut hits = engine::duckduckgo_search(http, &ddg_query, query.render, query.limit * 2)
        .await
        .unwrap_or_default();
    if hits.is_empty() {
        let mojeek_query = format!("{terms} {}", spec.domain);
        hits = engine::mojeek_search(http, &mojeek_query, query.render, query.limit * 3)
            .await
            .unwrap_or_default();
    }

    let out = hits
        .into_iter()
        .filter(|h| h.url.contains(spec.domain))
        .take(query.limit)
        .collect();
    Ok(out)
}

/// A documentation site exposed as a standalone `docs` provider.
pub(super) struct DocSiteProvider {
    spec: &'static DocSiteSpec,
}

#[async_trait]
impl SearchProvider for DocSiteProvider {
    fn id(&self) -> &'static str {
        self.spec.id
    }
    fn kind(&self) -> ProviderKind {
        ProviderKind::Docs
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        search(self.spec, http, query).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_builtins_resolve_with_their_domain() {
        assert_eq!(make("react").unwrap().spec.domain, "react.dev");
        assert_eq!(make("laravel").unwrap().spec.domain, "laravel.com");
        assert!(make("not-a-framework").is_none());
    }

    #[test]
    fn configured_docsite_uses_its_domain() {
        let p = make_configured("internal", "docs.internal.corp").unwrap();
        assert_eq!(p.spec.id, "internal");
        assert_eq!(p.spec.domain, "docs.internal.corp");
        // Missing domain is rejected.
        assert!(make_configured("x", "").is_none());
    }
}
