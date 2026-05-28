//! DuckDuckGo provider — scrapes the `lite.duckduckgo.com` HTML endpoint.
//! Honors `site:` so it doubles as a code provider (scoped to github.com).
//! DuckDuckGo rate-limits aggressively by IP, so it's typically paired with a
//! more tolerant fallback (e.g. Mojeek).

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};

use super::{finish, site_scoped_query, zip_links_snippets, HTML_ACCEPT};
use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};

pub(super) struct DuckDuckGo {
    pub(super) kind: ProviderKind,
}

#[async_trait]
impl SearchProvider for DuckDuckGo {
    fn id(&self) -> &'static str {
        "duckduckgo"
    }
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let q = if self.kind == ProviderKind::Code {
            site_scoped_query(query)
        } else {
            query.text.clone()
        };
        let hits = search_raw(http, &q, query.render, query.limit).await?;
        Ok(finish(self.kind, hits, query.limit, false))
    }
}

/// Run a raw DuckDuckGo search for an already-built query string and return the
/// parsed results (no kind-specific post-processing). Reused by forge providers.
pub(crate) async fn search_raw(
    http: &Client,
    query: &str,
    render: bool,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let body = fetch(http, query, render).await?;
    Ok(parse(&body, limit))
}

/// Non-render path POSTs to the lite endpoint (the known-good route). When the
/// caller requests rendering, the same query is loaded over GET in the headless
/// browser, which can slip past DuckDuckGo's IP rate-limiting.
async fn fetch(http: &Client, q: &str, render: bool) -> Result<String> {
    #[cfg(feature = "browser")]
    if render {
        use crate::browser::PageRenderer;
        let url = url::Url::parse_with_params("https://lite.duckduckgo.com/lite/", &[("q", q)])?;
        return crate::browser::shared_global().render(url.as_str()).await;
    }
    let _ = render;
    let body = http
        .post("https://lite.duckduckgo.com/lite/")
        .header("Accept", HTML_ACCEPT)
        .header("Accept-Language", "en-US,en;q=0.9")
        .form(&[("q", q)])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(body)
}

fn parse(body: &str, max: usize) -> Vec<SearchResult> {
    let doc = Html::parse_document(body);
    let link_sel = Selector::parse("a.result-link").unwrap();
    let snip_sel = Selector::parse("td.result-snippet").unwrap();
    zip_links_snippets(
        doc.select(&link_sel).collect(),
        doc.select(&snip_sel).collect(),
        max,
    )
}
