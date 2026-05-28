//! Mojeek provider — scrapes the `www.mojeek.com` HTML results. Mojeek runs its
//! own independent index and is far more tolerant of automated requests, making
//! it a reliable fallback. It does not support the `site:` operator, so for code
//! search we add "github.com" as a keyword and filter to GitHub URLs afterwards.

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

use super::{fetch_html, finish, keyword_scoped_query, zip_links_snippets};
use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};

pub(super) struct Mojeek {
    pub(super) kind: ProviderKind,
}

#[async_trait]
impl SearchProvider for Mojeek {
    fn id(&self) -> &'static str {
        "mojeek"
    }
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let q = if self.kind == ProviderKind::Code {
            keyword_scoped_query(query)
        } else {
            query.text.clone()
        };
        // Over-fetch for code so the GitHub filter still yields enough hits.
        let fetch = if self.kind == ProviderKind::Code {
            query.limit * 3
        } else {
            query.limit
        };
        let url = Url::parse_with_params("https://www.mojeek.com/search", &[("q", q.as_str())])?;
        let body = fetch_html(http, query, url.as_str()).await?;
        let hits = parse(&body, fetch);
        Ok(finish(self.kind, hits, query.limit, true))
    }
}

fn parse(body: &str, max: usize) -> Vec<SearchResult> {
    let doc = Html::parse_document(body);
    let link_sel = Selector::parse("a.title").unwrap();
    let snip_sel = Selector::parse("p.s").unwrap();
    zip_links_snippets(
        doc.select(&link_sel).collect(),
        doc.select(&snip_sel).collect(),
        max,
    )
}
