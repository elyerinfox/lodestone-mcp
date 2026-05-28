//! Concrete [`SearchProvider`] implementations, one per file, plus the
//! id → provider factory and helpers shared by the HTML-scraping engines.

mod apiengine;
mod bespoke;
mod composite;
mod docsite;
mod engine;
mod forge;
mod registry;

use std::sync::OnceLock;

use scraper::ElementRef;

use crate::config::Config;
use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::collapse_ws;

/// `Accept` header used by the HTML-scraping providers.
pub(crate) const HTML_ACCEPT: &str =
    "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8";

// ---------------------------------------------------------------------------
// Forge sites (code-search scope) — configurable so GitLab, Gitea/Codeberg,
// etc. can be searched alongside GitHub via the same web providers.
// ---------------------------------------------------------------------------

static CODE_SITES: OnceLock<Vec<String>> = OnceLock::new();

/// Set the forge domains that code search is scoped to. Call once at startup.
pub fn configure_code_sites(sites: Vec<String>) {
    let _ = CODE_SITES.set(sites);
}

fn code_sites() -> &'static [String] {
    CODE_SITES.get_or_init(|| vec!["github.com".to_string()])
}

/// Map a provider id to an instance, given the kind it's being used for and the
/// active config. (The same engine, e.g. duckduckgo, behaves differently for
/// web vs code.)
pub fn make(kind: ProviderKind, id: &str, cfg: &Config) -> Option<Box<dyn SearchProvider>> {
    use bespoke::{GrepApp, Medium, Searxng};
    use composite::{Github, StackExchange};

    match (kind, id) {
        // Spec-driven search engines (web + code), shared via HtmlEngineProvider.
        // `google` drives a headless browser; it just needs Chrome at runtime.
        (ProviderKind::Web, "duckduckgo" | "mojeek" | "google")
        | (ProviderKind::Code, "duckduckgo" | "mojeek" | "google") => {
            engine::make(kind, id).map(|p| Box::new(p) as Box<dyn SearchProvider>)
        }
        // Keyed web-search APIs (optional; active only when the key is configured).
        (ProviderKind::Web, "brave") => {
            let key = cfg.brave.key.clone();
            if key.is_empty() {
                tracing::warn!(
                    "brave web provider needs [brave].key (or LODESTONE_BRAVE_KEY); skipping"
                );
                None
            } else {
                apiengine::make("brave", key, Vec::new())
                    .map(|p| Box::new(p) as Box<dyn SearchProvider>)
            }
        }
        (ProviderKind::Web, "google_cse") => {
            let (key, cx) = (cfg.google_cse.key.clone(), cfg.google_cse.cx.clone());
            if key.is_empty() || cx.is_empty() {
                tracing::warn!("google_cse web provider needs [google_cse].key and .cx; skipping");
                None
            } else {
                apiengine::make("google_cse", key, vec![("cx".to_string(), cx)])
                    .map(|p| Box::new(p) as Box<dyn SearchProvider>)
            }
        }
        (ProviderKind::Web, "medium") => Some(Box::new(Medium)),
        (ProviderKind::Code, "grep_app") => Some(Box::new(GrepApp)),
        // SearXNG (web + code): a user-hosted meta-search instance. Inactive
        // unless [searxng].url is set.
        (ProviderKind::Web, "searxng") | (ProviderKind::Code, "searxng") => {
            let url = cfg.searxng.url.clone();
            if url.is_empty() {
                tracing::warn!("searxng listed as a provider but [searxng].url is empty; skipping");
                None
            } else {
                Some(Box::new(Searxng::new(url, kind)))
            }
        }
        // Composite GitHub: keyless scrape by default, authenticated API if a
        // token is configured.
        (ProviderKind::Code, "github") => Some(Box::new(Github::new(cfg.github.token.clone()))),
        // Spec-driven code forges, shared via ForgeCodeProvider.
        (ProviderKind::Code, "gitlab" | "codeberg" | "gitea") => {
            forge::make(id).map(|p| Box::new(p) as Box<dyn SearchProvider>)
        }
        (ProviderKind::Qa, "stackoverflow") | (ProviderKind::Qa, "stackexchange") => {
            Some(Box::new(StackExchange::new(cfg.stackexchange.key.clone())))
        }
        // Spec-driven documentation / package registries (keyless JSON APIs).
        (
            ProviderKind::Docs,
            "cratesio" | "npm" | "mdn" | "rubygems" | "packagist" | "nuget" | "hex" | "aur"
            | "dockerhub" | "archlinux",
        ) => registry::make(id).map(|p| Box::new(p) as Box<dyn SearchProvider>),
        // Built-in framework documentation sites (keyless, site-scoped web search).
        (ProviderKind::Docs, id) if docsite::make(id).is_some() => {
            docsite::make(id).map(|p| Box::new(p) as Box<dyn SearchProvider>)
        }
        // User-configured self-hosted forge (id defined under [forges]).
        (ProviderKind::Code, id) if cfg.forges.contains_key(id) => {
            let inst = &cfg.forges[id];
            forge::make_configured(id, &inst.kind, &inst.domain)
                .map(|p| Box::new(p) as Box<dyn SearchProvider>)
        }
        // User-configured documentation site (id defined under [docsites]).
        (ProviderKind::Docs, id) if cfg.docsites.contains_key(id) => {
            let inst = &cfg.docsites[id];
            docsite::make_configured(id, &inst.domain)
                .map(|p| Box::new(p) as Box<dyn SearchProvider>)
        }
        _ => None,
    }
}

/// Build a code query for engines that support the `site:` operator
/// (DuckDuckGo, Google): `<query> [lang] site:a OR site:b ...`.
pub(crate) fn site_scoped_query(query: &SearchQuery) -> String {
    let mut q = query.text.clone();
    if let Some(lang) = &query.language {
        q.push(' ');
        q.push_str(lang);
    }
    let scope = code_sites()
        .iter()
        .map(|s| format!("site:{s}"))
        .collect::<Vec<_>>()
        .join(" OR ");
    if !scope.is_empty() {
        q.push(' ');
        q.push_str(&scope);
    }
    q
}

/// Build a code query for engines without `site:` support (Mojeek): append the
/// forge domains as plain keywords; results are filtered to them afterwards.
pub(crate) fn keyword_scoped_query(query: &SearchQuery) -> String {
    let mut q = query.text.clone();
    if let Some(lang) = &query.language {
        q.push(' ');
        q.push_str(lang);
    }
    for site in code_sites() {
        q.push(' ');
        q.push_str(site);
    }
    q
}

/// Zip parallel lists of `<a>` title links and snippet elements into results.
pub(crate) fn zip_links_snippets(
    links: Vec<ElementRef>,
    snips: Vec<ElementRef>,
    max: usize,
) -> Vec<SearchResult> {
    let mut out = Vec::new();
    for (i, a) in links.iter().enumerate() {
        let url = a.value().attr("href").unwrap_or("").trim().to_string();
        if url.is_empty() {
            continue;
        }
        let title = collapse_ws(&a.text().collect::<String>());
        let snippet = snips
            .get(i)
            .map(|s| collapse_ws(&s.text().collect::<String>()))
            .unwrap_or_default();
        out.push(SearchResult {
            title,
            url,
            snippet,
            ..Default::default()
        });
        if out.len() >= max {
            break;
        }
    }
    out
}

/// Post-process raw web hits for the given kind: for code, optionally filter to
/// the configured forge domains and attach repo/path; truncate to `limit`.
pub(crate) fn finish(
    kind: ProviderKind,
    hits: Vec<SearchResult>,
    limit: usize,
    filter_to_sites: bool,
) -> Vec<SearchResult> {
    if kind != ProviderKind::Code {
        return hits;
    }
    let sites = code_sites();
    hits.into_iter()
        .filter(|h| !filter_to_sites || sites.iter().any(|s| h.url.contains(s.as_str())))
        .map(|mut h| {
            if let Some((repo, path)) = forge::repo_path(&h.url) {
                h.title = format!("{repo} — {path}");
                h.repo = Some(repo);
                h.path = Some(path);
            }
            h
        })
        .take(limit)
        .collect()
}
