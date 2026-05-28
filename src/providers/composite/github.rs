//! GitHub code search — a composite provider with two sourcing modes:
//!
//!   * **default (keyless): scrape** — a site-scoped web search of github.com,
//!     reusing the shared forge logic (`forge::search`), so `render` stays the
//!     caller's optional choice.
//!   * **token set: API** — GitHub's authenticated code-search API
//!     (`[github].token` / `GITHUB_TOKEN`). GitHub dropped unauthenticated code
//!     search, so the API is only reachable with a token; it's a strict opt-in
//!     enhancement over the keyless default.

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::providers::forge::{self, ForgeSpec};
use crate::retrieve::github_repo_path;
use crate::util::collapse_ws;

/// Spec for the keyless scrape path (shared forge machinery).
static SPEC: ForgeSpec = ForgeSpec {
    id: "github",
    domain: "github.com",
    repo_path: extract,
};

fn extract(url: &str) -> Option<(String, String)> {
    github_repo_path(url).map(|(repo, _branch, path)| (repo, path))
}

pub(crate) struct Github {
    token: String,
}

impl Github {
    pub(crate) fn new(token: String) -> Self {
        Self { token }
    }

    async fn search_api(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let mut q = query.text.clone();
        if let Some(lang) = &query.language {
            q.push_str(&format!(" language:{lang}"));
        }
        let per_page = query.limit.clamp(1, 50).to_string();
        let v: serde_json::Value = http
            .get("https://api.github.com/search/code")
            .query(&[("q", q.as_str()), ("per_page", per_page.as_str())])
            // text-match media type returns matching code fragments for snippets.
            .header("Accept", "application/vnd.github.text-match+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .bearer_auth(&self.token)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(parse_api(&v, query.limit))
    }
}

#[async_trait]
impl SearchProvider for Github {
    fn id(&self) -> &'static str {
        "github"
    }
    fn kind(&self) -> ProviderKind {
        ProviderKind::Code
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        if self.token.is_empty() {
            forge::search(&SPEC, http, query).await
        } else {
            self.search_api(http, query).await
        }
    }
}

fn parse_api(v: &serde_json::Value, max: usize) -> Vec<SearchResult> {
    let items = match v.get("items").and_then(|i| i.as_array()) {
        Some(i) => i,
        None => return vec![],
    };
    let mut out = Vec::new();
    for item in items {
        let repo = item
            .pointer("/repository/full_name")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let path = item.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let url = item.get("html_url").and_then(|x| x.as_str()).unwrap_or("");
        let snippet = item
            .get("text_matches")
            .and_then(|m| m.as_array())
            .and_then(|m| m.first())
            .and_then(|m| m.get("fragment"))
            .and_then(|x| x.as_str())
            .map(collapse_ws)
            .unwrap_or_default();
        out.push(SearchResult {
            title: if path.is_empty() {
                repo.to_string()
            } else {
                format!("{repo} — {path}")
            },
            url: url.to_string(),
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
    fn parse_api_extracts_repo_path_url_snippet() {
        let v = serde_json::json!({
            "items": [{
                "repository": {"full_name": "rust-lang/rust"},
                "path": "src/lib.rs",
                "html_url": "https://github.com/rust-lang/rust/blob/master/src/lib.rs",
                "text_matches": [{"fragment": "fn  main() {}"}]
            }]
        });
        let out = super::parse_api(&v, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].repo.as_deref(), Some("rust-lang/rust"));
        assert_eq!(out[0].path.as_deref(), Some("src/lib.rs"));
        assert_eq!(
            out[0].url,
            "https://github.com/rust-lang/rust/blob/master/src/lib.rs"
        );
        assert_eq!(out[0].snippet, "fn main() {}"); // collapse_ws normalizes spacing
        assert_eq!(out[0].title, "rust-lang/rust — src/lib.rs");
    }

    #[test]
    fn parse_api_missing_items_is_empty() {
        assert!(super::parse_api(&serde_json::json!({}), 10).is_empty());
    }
}
