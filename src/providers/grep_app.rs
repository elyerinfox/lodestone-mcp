//! grep.app provider — queries grep.app's JSON code-search endpoint. grep.app is
//! frequently behind a bot-challenge that returns HTML instead of JSON; in that
//! case this provider returns no results so the registry falls through to the
//! next code provider.

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::html_to_text;

pub(super) struct GrepApp;

#[async_trait]
impl SearchProvider for GrepApp {
    fn id(&self) -> &'static str {
        "grep_app"
    }
    fn kind(&self) -> ProviderKind {
        ProviderKind::Code
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let resp = http
            .get("https://grep.app/api/search")
            .query(&[("q", query.text.as_str())])
            .header("Accept", "application/json")
            .header("Referer", "https://grep.app/")
            .send()
            .await?;
        if !resp.status().is_success() {
            return Ok(vec![]);
        }
        let text = resp.text().await?;
        Ok(parse(&text, query.limit))
    }
}

fn parse(text: &str, max: usize) -> Vec<SearchResult> {
    let v: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return vec![], // bot-challenge HTML rather than JSON
    };
    let hits = match v.pointer("/hits/hits").and_then(|h| h.as_array()) {
        Some(h) => h,
        None => return vec![],
    };
    let mut out = Vec::new();
    for h in hits {
        let repo = h
            .pointer("/repo/raw")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let path = h
            .pointer("/path/raw")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let branch = h
            .pointer("/branch/raw")
            .and_then(|x| x.as_str())
            .unwrap_or("HEAD");
        let snippet = h
            .pointer("/content/snippet")
            .and_then(|x| x.as_str())
            .map(html_to_text)
            .unwrap_or_default();
        let url = if !repo.is_empty() && !path.is_empty() {
            format!("https://github.com/{repo}/blob/{branch}/{path}")
        } else {
            String::new()
        };
        out.push(SearchResult {
            title: if path.is_empty() {
                repo.to_string()
            } else {
                format!("{repo} — {path}")
            },
            url,
            snippet,
            repo: (!repo.is_empty()).then(|| repo.to_string()),
            path: (!path.is_empty()).then(|| path.to_string()),
            ..Default::default()
        });
        if out.len() >= max {
            break;
        }
    }
    out
}
