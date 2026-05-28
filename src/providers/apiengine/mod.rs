//! Authenticated JSON web-search engines (keyed APIs).
//!
//! Spec-driven family for web search that *requires an API key*. Per golden rule
//! 3 these stay optional: a provider is constructed only when its key is set, is
//! off by default, and never replaces the keyless providers — it's a strictly
//! optional enhancement. Each engine is a GET JSON API differing only in
//! declarative specifics ([`ApiSpec`]: endpoint, query/size params, how the key is
//! sent ([`Auth`]), results pointer, field pointers); the shared [`ApiProvider`]
//! runs it. Render is n/a for a JSON API (the flag is accepted and ignored).

mod brave;
mod google;

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::collapse_ws;

/// How the API key is transmitted.
pub(super) enum Auth {
    /// Sent as an HTTP header of this name.
    Header(&'static str),
    /// Sent as a query parameter of this name.
    Query(&'static str),
}

/// Declarative description of a keyed JSON web-search API.
pub(super) struct ApiSpec {
    pub id: &'static str,
    pub url: &'static str,
    pub query_key: &'static str,
    /// Result-count parameter (set to min(requested, `size_cap`)).
    pub size_key: Option<&'static str>,
    /// API-imposed maximum results per request (e.g. Brave 20, Google CSE 10).
    pub size_cap: usize,
    pub auth: Auth,
    pub extra_params: &'static [(&'static str, &'static str)],
    pub results_ptr: &'static str,
    pub title: &'static str,
    pub link: &'static str,
    pub snippet: &'static str,
}

/// Construct a keyed web provider by id. `key` is its API key; `extra` carries any
/// additional credential query params (e.g. Google's `cx`). The caller has already
/// gated on a non-empty key.
pub(super) fn make(id: &str, key: String, extra: Vec<(String, String)>) -> Option<ApiProvider> {
    let spec = match id {
        "brave" => &brave::SPEC,
        "google_cse" => &google::SPEC,
        _ => return None,
    };
    Some(ApiProvider { spec, key, extra })
}

pub(super) struct ApiProvider {
    spec: &'static ApiSpec,
    key: String,
    extra: Vec<(String, String)>,
}

#[async_trait]
impl SearchProvider for ApiProvider {
    fn id(&self) -> &'static str {
        self.spec.id
    }
    fn kind(&self) -> ProviderKind {
        ProviderKind::Web
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let spec = self.spec;
        let size = query.limit.min(spec.size_cap).max(1).to_string();
        let mut params: Vec<(&str, &str)> = vec![(spec.query_key, query.text.as_str())];
        if let Some(k) = spec.size_key {
            params.push((k, size.as_str()));
        }
        params.extend_from_slice(spec.extra_params);
        for (k, v) in &self.extra {
            params.push((k.as_str(), v.as_str()));
        }
        if let Auth::Query(p) = &spec.auth {
            params.push((p, self.key.as_str()));
        }

        let mut req = http
            .get(spec.url)
            .query(&params)
            .header("Accept", "application/json");
        if let Auth::Header(h) = &spec.auth {
            req = req.header(*h, &self.key);
        }

        let v: Value = req.send().await?.error_for_status()?.json().await?;
        Ok(parse(spec, &v, query.limit))
    }
}

fn parse(spec: &ApiSpec, v: &Value, max: usize) -> Vec<SearchResult> {
    let items = match v.pointer(spec.results_ptr).and_then(|x| x.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for item in items {
        let url = item
            .pointer(spec.link)
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if url.is_empty() {
            continue;
        }
        let title = item
            .pointer(spec.title)
            .and_then(|x| x.as_str())
            .map(collapse_ws)
            .unwrap_or_default();
        let snippet = item
            .pointer(spec.snippet)
            .and_then(|x| x.as_str())
            .map(collapse_ws)
            .unwrap_or_default();
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
    use super::*;

    #[test]
    fn brave_parse_web_results() {
        let v = serde_json::json!({
            "web": {"results": [
                {"title": "Rust", "url": "https://rust-lang.org", "description": "  the lang  "}
            ]}
        });
        let out = parse(&brave::SPEC, &v, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://rust-lang.org");
        assert_eq!(out[0].title, "Rust");
        assert_eq!(out[0].snippet, "the lang");
    }

    #[test]
    fn google_cse_parse_items() {
        let v = serde_json::json!({
            "items": [
                {"title": "Tokio", "link": "https://tokio.rs", "snippet": "async runtime"}
            ]
        });
        let out = parse(&google::SPEC, &v, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://tokio.rs");
        assert_eq!(out[0].title, "Tokio");
    }

    #[test]
    fn missing_results_is_empty() {
        assert!(parse(&brave::SPEC, &serde_json::json!({}), 10).is_empty());
    }
}
