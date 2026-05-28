//! Google provider (feature `google`) — scrapes google.com using the shared
//! headless browser (see [`crate::browser`]) so the request looks like a real
//! browser. Google still shows CAPTCHAs from repeated/datacenter IPs and a
//! regional consent page can hide results, so keep a tolerant provider (Mojeek)
//! in the chain as a fallback.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

use super::{finish, site_scoped_query};
use crate::browser::PageRenderer;
use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::collapse_ws;

pub(super) struct Google {
    kind: ProviderKind,
}

impl Google {
    pub(super) fn new(kind: ProviderKind) -> Self {
        Self { kind }
    }

    async fn render(&self, query: &str, limit: usize) -> Result<String> {
        let num = limit.clamp(1, 20).to_string();
        let url = Url::parse_with_params(
            "https://www.google.com/search",
            &[("q", query), ("num", num.as_str()), ("hl", "en"), ("gl", "us")],
        )?;
        crate::browser::shared_global().render(url.as_str()).await
    }
}

#[async_trait]
impl SearchProvider for Google {
    fn id(&self) -> &'static str {
        "google"
    }
    fn kind(&self) -> ProviderKind {
        self.kind
    }
    async fn search(&self, _http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let text = if self.kind == ProviderKind::Code {
            site_scoped_query(query)
        } else {
            query.text.clone()
        };
        let html = self.render(&text, query.limit).await?;
        let hits = parse(&html, query.limit);
        if hits.is_empty() && looks_blocked(&html) {
            return Err(anyhow!(
                "Google returned a CAPTCHA / consent page instead of results"
            ));
        }
        let filter_github = self.kind == ProviderKind::Code;
        Ok(finish(self.kind, hits, query.limit, filter_github))
    }
}

fn parse(html: &str, max: usize) -> Vec<SearchResult> {
    let doc = Html::parse_document(html);
    let h3_sel = Selector::parse("h3").unwrap();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for h3 in doc.select(&h3_sel) {
        // Walk up to the enclosing result anchor.
        let mut href = None;
        for ancestor in h3.ancestors() {
            if let Some(el) = ancestor.value().as_element() {
                if el.name() == "a" {
                    if let Some(h) = el.attr("href") {
                        href = Some(h.to_string());
                        break;
                    }
                }
            }
        }
        let url = match href.as_deref().and_then(clean_href) {
            Some(u) => u,
            None => continue,
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let title = collapse_ws(&h3.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        out.push(SearchResult {
            title,
            url,
            ..Default::default()
        });
        if out.len() >= max {
            break;
        }
    }
    out
}

/// Heuristic: does this look like Google's anti-bot / consent interstitial
/// rather than a results page?
fn looks_blocked(html: &str) -> bool {
    let h = html.to_ascii_lowercase();
    h.contains("/sorry/")
        || h.contains("unusual traffic")
        || h.contains("recaptcha")
        || h.contains("our systems have detected")
        || h.contains("before you continue to google")
}

/// Normalize a Google result href to a real destination URL, dropping internal
/// links. Handles both direct hrefs and the legacy `/url?q=` redirector.
fn clean_href(href: &str) -> Option<String> {
    if href.starts_with("http") {
        if href.contains("google.com") {
            return None;
        }
        return Some(href.to_string());
    }
    if let Some(rest) = href.strip_prefix("/url?") {
        for (k, v) in url::form_urlencoded::parse(rest.as_bytes()) {
            if k == "q" {
                return Some(v.into_owned());
            }
        }
    }
    None
}
