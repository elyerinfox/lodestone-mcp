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

/// Declarative description of a web search engine.
pub(super) struct EngineSpec {
    pub id: &'static str,
    /// Search endpoint URL (the query is sent as `q`).
    pub url: &'static str,
    pub method: Method,
    pub extract: Extract,
    pub code_scope: CodeScope,
    /// Extra fixed query parameters (e.g. Google's `hl`/`gl`/`num`).
    pub extra_params: &'static [(&'static str, &'static str)],
}

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

/// Fetch + parse for an already-built query string (no kind-specific scoping).
/// One short backoff retry guards against transient timeouts / IP throttling,
/// which engines like DuckDuckGo do frequently under load.
async fn search_raw(
    spec: &EngineSpec,
    http: &Client,
    query: &str,
    render: bool,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let body = match fetch(spec, http, query, render).await {
        Ok(body) => body,
        Err(first) => {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            // Keep the original error if the retry also fails.
            fetch(spec, http, query, render).await.map_err(|_| first)?
        }
    };
    Ok(parse(spec, &body, limit))
}

async fn fetch(spec: &EngineSpec, http: &Client, q: &str, render: bool) -> Result<String> {
    let mut params: Vec<(&str, &str)> = vec![("q", q)];
    params.extend_from_slice(spec.extra_params);

    // Engines whose method is Browser always render; any engine renders when the
    // caller asks for it.
    if matches!(spec.method, Method::Browser) || render {
        use crate::browser::PageRenderer;
        let url = url::Url::parse_with_params(spec.url, &params)?;
        return crate::browser::shared_global().render(url.as_str()).await;
    }

    let request = match spec.method {
        Method::Get => http.get(spec.url).query(&params),
        Method::PostForm => http
            .post(spec.url)
            .query(spec.extra_params)
            .form(&[("q", q)]),
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

fn parse(spec: &EngineSpec, body: &str, max: usize) -> Vec<SearchResult> {
    match &spec.extract {
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
