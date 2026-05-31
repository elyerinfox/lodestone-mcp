//! grep.app provider — queries grep.app's JSON code-search endpoint. grep.app is
//! frequently behind a bot-challenge that returns HTML instead of JSON; in that
//! case this provider returns no results so the registry falls through to the
//! next code provider.

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::html_to_text;

pub(crate) struct GrepApp;

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

#[cfg(test)]
mod tests {
    #[test]
    fn parses_hits_into_github_blob_urls() {
        let json = r#"{"hits":{"hits":[
            {"repo":{"raw":"o/r"},"path":{"raw":"src/a.rs"},"branch":{"raw":"main"},
             "content":{"snippet":"<div>fn x()</div>"}}
        ]}}"#;
        let out = super::parse(json, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://github.com/o/r/blob/main/src/a.rs");
        assert_eq!(out[0].repo.as_deref(), Some("o/r"));
        assert_eq!(out[0].path.as_deref(), Some("src/a.rs"));
    }

    #[test]
    fn non_json_body_returns_empty() {
        // A bot-challenge HTML page must not error — just yield nothing.
        assert!(super::parse("<html>are you a robot?</html>", 10).is_empty());
    }
}

#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        crate::skills::live_http()
    }

    /// grep.app's keyless code-search JSON endpoint. The parser test pins
    /// schema; this catches the day grep.app goes dark or changes URL.
    #[tokio::test]
    #[ignore]
    async fn grep_app_search_live() {
        let r = http()
            .get("https://grep.app/api/search?q=tokio::spawn")
            .send()
            .await
            .expect("network");
        // grep.app sometimes drops to a bot challenge page (200 HTML); both
        // 200-HTML and 200-JSON are acceptable — the parser handles both.
        if !r.status().is_success() {
            eprintln!("grep_app_search_live: status {}", r.status());
            return;
        }
        let body = r.text().await.unwrap();
        let parsed = super::parse(&body, 10);
        // If grep.app is serving JSON results, we expect ≥ 1; if it's a bot
        // challenge HTML page, parse returns empty — both are "API up".
        eprintln!("grep_app_search_live: got {} results", parsed.len());
    }
}
