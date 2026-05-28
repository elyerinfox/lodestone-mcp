//! Concrete [`SearchProvider`] implementations, one per file, plus the
//! id → provider factory and helpers shared by the HTML-scraping engines.

mod duckduckgo;
mod forge;
mod github_api;
#[cfg(feature = "google")]
mod google;
mod grep_app;
mod medium;
mod mojeek;
mod stackexchange;

use std::sync::OnceLock;

use anyhow::Result;
use reqwest::Client;
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
    use duckduckgo::DuckDuckGo;
    use github_api::GithubApi;
    use grep_app::GrepApp;
    use medium::Medium;
    use mojeek::Mojeek;
    use stackexchange::StackExchange;

    match (kind, id) {
        (ProviderKind::Web, "duckduckgo") => Some(Box::new(DuckDuckGo { kind })),
        (ProviderKind::Web, "mojeek") => Some(Box::new(Mojeek { kind })),
        (ProviderKind::Web, "medium") => Some(Box::new(Medium)),
        (ProviderKind::Code, "grep_app") => Some(Box::new(GrepApp)),
        (ProviderKind::Code, "duckduckgo") => Some(Box::new(DuckDuckGo { kind })),
        (ProviderKind::Code, "mojeek") => Some(Box::new(Mojeek { kind })),
        (ProviderKind::Code, "github_web")
        | (ProviderKind::Code, "gitlab")
        | (ProviderKind::Code, "codeberg")
        | (ProviderKind::Code, "gitea") => {
            forge::make(id).map(|p| Box::new(p) as Box<dyn SearchProvider>)
        }
        (ProviderKind::Code, "github") => {
            if cfg.github.token.is_empty() {
                tracing::warn!(
                    "`github` code provider needs a token (config [github].token or \
                     GITHUB_TOKEN); skipping"
                );
                None
            } else {
                Some(Box::new(GithubApi::new(cfg.github.token.clone())))
            }
        }
        (ProviderKind::Qa, "stackoverflow") | (ProviderKind::Qa, "stackexchange") => {
            Some(Box::new(StackExchange::new(cfg.stackexchange.key.clone())))
        }
        #[cfg(feature = "google")]
        (ProviderKind::Web, "google") | (ProviderKind::Code, "google") => {
            Some(Box::new(google::Google::new(kind)))
        }
        _ => None,
    }
}

/// GET a URL and return its HTML. If the caller requested rendering (and the
/// `browser` feature is built in), the page is loaded in the shared headless
/// browser; otherwise a plain HTTP request is used. This is what lets the model
/// opt any HTML-scraping provider into browser rendering per call.
pub(crate) async fn fetch_html_render(http: &Client, render: bool, url: &str) -> Result<String> {
    #[cfg(feature = "browser")]
    if render {
        use crate::browser::PageRenderer;
        return crate::browser::shared_global().render(url).await;
    }
    let _ = render;
    let body = http
        .get(url)
        .header("Accept", HTML_ACCEPT)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(body)
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
