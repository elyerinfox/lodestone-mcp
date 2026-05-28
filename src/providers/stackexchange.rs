//! StackExchange provider — the keyless public Search/Advanced API
//! (`api.stackexchange.com`). No token required; unauthenticated calls share a
//! per-IP daily quota (~300 requests).

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::decode_entities;

pub(super) struct StackExchange;

#[async_trait]
impl SearchProvider for StackExchange {
    fn id(&self) -> &'static str {
        "stackoverflow"
    }
    fn kind(&self) -> ProviderKind {
        ProviderKind::Qa
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let site = query.site.as_deref().unwrap_or("stackoverflow");
        let pagesize = query.limit.clamp(1, 50).to_string();
        let v: serde_json::Value = http
            .get("https://api.stackexchange.com/2.3/search/advanced")
            .query(&[
                ("order", "desc"),
                ("sort", "relevance"),
                ("q", query.text.as_str()),
                ("site", site),
                ("pagesize", pagesize.as_str()),
                ("filter", "default"),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(parse(&v, query.limit))
    }
}

fn parse(v: &serde_json::Value, max: usize) -> Vec<SearchResult> {
    let items = match v.get("items").and_then(|i| i.as_array()) {
        Some(i) => i,
        None => return vec![],
    };
    let mut out = Vec::new();
    for q in items {
        let title = decode_entities(q.get("title").and_then(|x| x.as_str()).unwrap_or("(untitled)"));
        let link = q.get("link").and_then(|x| x.as_str()).unwrap_or("");
        let score = q.get("score").and_then(|x| x.as_i64());
        let answers = q.get("answer_count").and_then(|x| x.as_i64()).unwrap_or(0);
        let accepted = q.get("accepted_answer_id").is_some();
        let tags = q
            .get("tags")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|t| t.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let meta = format!(
            "{answers} answers{}{}",
            if accepted { " · accepted ✓" } else { "" },
            if tags.is_empty() {
                String::new()
            } else {
                format!(" · tags: {tags}")
            }
        );
        out.push(SearchResult {
            title,
            url: link.to_string(),
            snippet: String::new(),
            score,
            meta: Some(meta),
            ..Default::default()
        });
        if out.len() >= max {
            break;
        }
    }
    out
}
