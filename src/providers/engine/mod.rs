//! Web search engines.
//!
//! Like `forge`, this is a spec-driven family: every web search engine shares
//! the SAME logic — build the query, fetch it, parse results — and differs only
//! in DECLARATIVE specifics captured by [`EngineSpec`]: the endpoint, how the
//! query is sent ([`Method`] — GET, POST form, or always-render via the headless
//! browser), how results are extracted ([`Extract`] — two CSS selectors for the
//! simple case, or a custom parser for messy layouts like Google), and how code
//! search scopes to forge domains ([`CodeScope`]). Adding an engine is one small
//! file (`duckduckgo.rs`, `mojeek.rs`, `google.rs`) declaring a spec; the shared
//! [`HtmlEngineProvider`] turns it into a working web+code provider.

mod duckduckgo;
mod google;
mod mojeek;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};

/// How the query is sent to the engine.
pub(super) enum Method {
    /// GET with `?q=…`.
    Get,
    /// POST the query as a form field (e.g. DuckDuckGo lite).
    PostForm,
    /// Always load the page in the headless browser (e.g. Google).
    Browser,
}

/// How results are pulled out of the fetched page.
pub(super) enum Extract {
    /// Zip two CSS selectors: result title anchors and their snippets.
    Selectors {
        link: &'static str,
        snippet: &'static str,
    },
    /// A custom parser, for engines whose markup needs real logic (e.g. Google).
    Custom(fn(&str, usize) -> Vec<SearchResult>),
}

/// How code search is scoped to the configured forge domains.
pub(super) enum CodeScope {
    /// Engine supports the `site:` operator (e.g. DuckDuckGo, Google).
    SiteOperator,
    /// No `site:`; append domains as keywords and filter results (e.g. Mojeek).
    Keyword,
}

/// One concrete way to reach an engine: a URL with its own send method and
/// result-extraction rule. An engine's primary lives inline on [`EngineSpec`];
/// `alts` lists interchangeable mirrors (e.g. DuckDuckGo's `lite` vs `html`).
pub(super) struct Endpoint {
    pub url: &'static str,
    pub method: Method,
    pub extract: Extract,
}

/// Declarative description of a web search engine.
pub(super) struct EngineSpec {
    pub id: &'static str,
    /// Primary search endpoint URL (the query is sent as `q`).
    pub url: &'static str,
    pub method: Method,
    pub extract: Extract,
    /// Interchangeable alternate endpoints, tried (and rotated through) when the
    /// primary errors or returns nothing — e.g. DuckDuckGo's `html` mirror behind
    /// the `lite` primary. Empty for engines with a single endpoint.
    pub alts: &'static [Endpoint],
    pub code_scope: CodeScope,
    /// Extra fixed query parameters (e.g. Google's `hl`/`gl`/`num`).
    pub extra_params: &'static [(&'static str, &'static str)],
}

/// Round-robin offset so successive calls to a multi-endpoint engine start at a
/// different endpoint — spreading load (and IP-throttling) across mirrors.
static ROTATE: AtomicUsize = AtomicUsize::new(0);

/// Construct an engine provider by id, for the given kind.
pub(super) fn make(kind: ProviderKind, id: &str) -> Option<HtmlEngineProvider> {
    let spec = match id {
        "duckduckgo" => &duckduckgo::SPEC,
        "mojeek" => &mojeek::SPEC,
        "google" => &google::SPEC,
        _ => return None,
    };
    Some(HtmlEngineProvider { spec, kind })
}

/// Run DuckDuckGo for an already-built query string (used by forge providers).
pub(super) async fn duckduckgo_search(
    http: &Client,
    query: &str,
    render: bool,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    search_raw(&duckduckgo::SPEC, http, query, render, limit).await
}

/// Run Mojeek for an already-built query string (used by forge providers).
pub(super) async fn mojeek_search(
    http: &Client,
    query: &str,
    render: bool,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    search_raw(&mojeek::SPEC, http, query, render, limit).await
}

/// Shared web/code provider, driven entirely by an [`EngineSpec`].
pub(super) struct HtmlEngineProvider {
    spec: &'static EngineSpec,
    kind: ProviderKind,
}

#[async_trait]
impl SearchProvider for HtmlEngineProvider {
    fn id(&self) -> &'static str {
        self.spec.id
    }
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let (q, fetch_limit, filter_to_sites) = if self.kind == ProviderKind::Code {
            match self.spec.code_scope {
                CodeScope::SiteOperator => (super::site_scoped_query(query), query.limit, false),
                CodeScope::Keyword => (super::keyword_scoped_query(query), query.limit * 3, true),
            }
        } else {
            (query.text.clone(), query.limit, false)
        };

        let hits = search_raw(self.spec, http, &q, query.render, fetch_limit).await?;
        Ok(super::finish(self.kind, hits, query.limit, filter_to_sites))
    }
}

/// One reachable endpoint of a spec: its URL, send method, and extraction rule.
struct Attempt<'a> {
    url: &'a str,
    method: &'a Method,
    extract: &'a Extract,
}

/// Fetch + parse for an already-built query string (no kind-specific scoping).
///
/// Endpoints (the primary plus any `alts`) are tried in a **rotated** order so
/// load spreads across mirrors, and an endpoint that errors or returns nothing is
/// followed (after a short, growing **backoff**) by the next — both guarding
/// against the transient timeouts / IP throttling engines like DuckDuckGo do under
/// load. The first non-empty result wins; an empty-but-successful result is kept as
/// a fallback so a genuinely empty search still returns `Ok([])`.
async fn search_raw(
    spec: &EngineSpec,
    http: &Client,
    query: &str,
    render: bool,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let mut attempts: Vec<Attempt> = Vec::with_capacity(1 + spec.alts.len());
    attempts.push(Attempt {
        url: spec.url,
        method: &spec.method,
        extract: &spec.extract,
    });
    for e in spec.alts {
        attempts.push(Attempt {
            url: e.url,
            method: &e.method,
            extract: &e.extract,
        });
    }

    let n = attempts.len();
    let start = if n > 1 {
        ROTATE.fetch_add(1, Ordering::Relaxed) % n
    } else {
        0
    };

    let mut last_ok: Option<Vec<SearchResult>> = None;
    let mut last_err: Option<anyhow::Error> = None;
    for off in 0..n {
        let a = &attempts[(start + off) % n];
        match try_endpoint(a, spec.extra_params, http, query, render, limit).await {
            Ok(results) if !results.is_empty() => return Ok(results),
            Ok(results) => last_ok = Some(results),
            Err(e) => last_err = Some(e),
        }
        if off + 1 < n {
            // Growing backoff between endpoints (250ms, 500ms, …).
            tokio::time::sleep(Duration::from_millis(250 * (off as u64 + 1))).await;
        }
    }
    match last_ok {
        Some(results) => Ok(results),
        None => Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no endpoints configured"))),
    }
}

/// Hit one endpoint, with a single in-place retry on a transient fetch error.
async fn try_endpoint(
    a: &Attempt<'_>,
    extra_params: &[(&str, &str)],
    http: &Client,
    query: &str,
    render: bool,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let body = match fetch(a.url, a.method, extra_params, http, query, render).await {
        Ok(body) => body,
        Err(first) => {
            tokio::time::sleep(Duration::from_millis(300)).await;
            // Keep the original error if the retry also fails.
            fetch(a.url, a.method, extra_params, http, query, render)
                .await
                .map_err(|_| first)?
        }
    };
    Ok(parse(a.extract, &body, limit))
}

async fn fetch(
    url: &str,
    method: &Method,
    extra_params: &[(&str, &str)],
    http: &Client,
    q: &str,
    render: bool,
) -> Result<String> {
    let mut params: Vec<(&str, &str)> = vec![("q", q)];
    params.extend_from_slice(extra_params);

    // Engines whose method is Browser always render; any engine renders when the
    // caller asks for it.
    if matches!(method, Method::Browser) || render {
        use crate::browser::PageRenderer;
        let url = url::Url::parse_with_params(url, &params)?;
        return crate::browser::shared_global().render(url.as_str()).await;
    }

    let request = match method {
        Method::Get => http.get(url).query(&params),
        Method::PostForm => http.post(url).query(extra_params).form(&[("q", q)]),
        Method::Browser => unreachable!("Browser method handled above"),
    };
    let body = request
        .header("Accept", super::HTML_ACCEPT)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(body)
}

fn parse(extract: &Extract, body: &str, max: usize) -> Vec<SearchResult> {
    match extract {
        Extract::Selectors { link, snippet } => {
            let doc = Html::parse_document(body);
            let link_sel = Selector::parse(link).unwrap();
            let snip_sel = Selector::parse(snippet).unwrap();
            super::zip_links_snippets(
                doc.select(&link_sel).collect(),
                doc.select(&snip_sel).collect(),
                max,
            )
        }
        Extract::Custom(parser) => parser(body, max),
    }
}

#[cfg(test)]
mod tests {
    // DDG lite is a table layout, so snippet cells must live inside a <table>
    // (html5ever drops stray <td>s otherwise).
    #[test]
    fn selector_engine_parses_links_and_snippets() {
        let html = r#"<html><body><table>
            <tr><td><a class="result-link" href="https://a.example/x">Alpha</a></td></tr>
            <tr><td class="result-snippet">snippet a</td></tr>
            <tr><td><a class="result-link" href="https://b.example/y">Beta</a></td></tr>
            <tr><td class="result-snippet">snippet b</td></tr>
        </table></body></html>"#;
        let out = super::parse(&super::duckduckgo::SPEC.extract, html, 10);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].url, "https://a.example/x");
        assert_eq!(out[0].title, "Alpha");
        assert_eq!(out[0].snippet, "snippet a");
        assert_eq!(out[1].url, "https://b.example/y");
    }

    #[test]
    fn selector_engine_respects_max() {
        let html = concat!(
            r#"<table><tr><td><a class="result-link" href="https://a">A</a></td></tr>"#,
            r#"<tr><td class="result-snippet">s</td></tr>"#,
            r#"<tr><td><a class="result-link" href="https://b">B</a></td></tr>"#,
            r#"<tr><td class="result-snippet">s</td></tr></table>"#,
        );
        assert_eq!(
            super::parse(&super::duckduckgo::SPEC.extract, html, 1).len(),
            1
        );
    }
}
