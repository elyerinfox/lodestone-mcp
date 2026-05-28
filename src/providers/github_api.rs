//! GitHub code-search provider using the authenticated REST API. GitHub no
//! longer permits unauthenticated code search, so this provider is only used
//! when a token is configured (`[github].token` or `GITHUB_TOKEN`). A classic
//! token or a fine-grained token with read access works. Without a token, code
//! search falls back to the keyless site-scoped web providers.

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::collapse_ws;

pub(super) struct GithubApi {
    token: String,
}

impl GithubApi {
    pub(super) fn new(token: String) -> Self {
        Self { token }
    }
}

#[async_trait]
impl SearchProvider for GithubApi {
    fn id(&self) -> &'static str {
        "github"
    }
    fn kind(&self) -> ProviderKind {
        ProviderKind::Code
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
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
        Ok(parse(&v, query.limit))
    }
}

fn parse(v: &serde_json::Value, max: usize) -> Vec<SearchResult> {
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
