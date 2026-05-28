//! SearXNG provider (web + code). Queries a user-hosted SearXNG instance's JSON
//! API (`{url}/search?format=json`), which aggregates many upstream engines —
//! the strongest keyless option for users willing to run one. Inactive unless
//! `[searxng].url` is set. For code search the query is `site:`-scoped to the
//! configured forge domains, exactly like the HTML engines.

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::collapse_ws;

pub(crate) struct Searxng {
    base_url: String,
    kind: ProviderKind,
}

impl Searxng {
    pub(crate) fn new(base_url: String, kind: ProviderKind) -> Self {
        Self { base_url, kind }
    }
}

#[async_trait]
impl SearchProvider for Searxng {
    fn id(&self) -> &'static str {
        "searxng"
    }
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        if self.base_url.is_empty() {
            return Ok(vec![]);
        }
        let q = if self.kind == ProviderKind::Code {
            crate::providers::site_scoped_query(query)
        } else {
            query.text.clone()
        };
        let endpoint = format!("{}/search", self.base_url.trim_end_matches('/'));
        let v: serde_json::Value = http
            .get(&endpoint)
            .query(&[("q", q.as_str()), ("format", "json")])
            .header("Accept", "application/json")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let hits = parse(&v, query.limit);
        Ok(crate::providers::finish(
            self.kind,
            hits,
            query.limit,
            false,
        ))
    }
}

fn parse(v: &serde_json::Value, max: usize) -> Vec<SearchResult> {
    let results = match v.get("results").and_then(|r| r.as_array()) {
        Some(r) => r,
        None => return vec![],
    };
    let mut out = Vec::new();
    for r in results {
        let url = r.get("url").and_then(|x| x.as_str()).unwrap_or("").trim();
        if url.is_empty() {
            continue;
        }
        let title = collapse_ws(r.get("title").and_then(|x| x.as_str()).unwrap_or(""));
        let snippet = collapse_ws(r.get("content").and_then(|x| x.as_str()).unwrap_or(""));
        out.push(SearchResult {
            title,
            url: url.to_string(),
            snippet,
            ..Default::default()
        });
        if out.len() >= max {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_results_array() {
        let v = serde_json::json!({
            "results": [
                {"url": "https://example.com/a", "title": "A", "content": "first"},
                {"url": "https://example.com/b", "title": "B", "content": "second"},
                {"title": "no url, skipped"}
            ]
        });
        let out = super::parse(&v, 10);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].url, "https://example.com/a");
        assert_eq!(out[0].title, "A");
        assert_eq!(out[1].snippet, "second");
    }

    #[test]
    fn missing_results_is_empty() {
        assert!(super::parse(&serde_json::json!({}), 10).is_empty());
    }
}
