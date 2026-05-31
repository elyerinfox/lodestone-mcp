//! Medium provider (web). Medium's search page is JS/bot-walled, but its tag
//! RSS feeds are keyless and stable, so this provider treats the query as a
//! Medium tag and returns recent articles from `medium.com/feed/tag/<tag>`.
//! It reads recent posts for a topic rather than doing full-text relevance
//! search.

use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use reqwest::Client;

use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::{decode_entities, html_to_text, truncate_chars};

static ITEM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<item>(.*?)</item>").unwrap());
static TITLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<title>(.*?)</title>").unwrap());
static LINK_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<link>(.*?)</link>").unwrap());
static DESC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<description>(.*?)</description>").unwrap());

pub(crate) struct Medium;

#[async_trait::async_trait]
impl SearchProvider for Medium {
    fn id(&self) -> &'static str {
        "medium"
    }
    fn kind(&self) -> ProviderKind {
        ProviderKind::Web
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let tag = tag_slug(&query.text);
        if tag.is_empty() {
            return Ok(vec![]);
        }
        let xml = http
            .get(format!("https://medium.com/feed/tag/{tag}"))
            .header("Accept", "application/rss+xml, application/xml, text/xml")
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        Ok(parse(&xml, query.limit))
    }
}

/// Turn a free-text query into a Medium tag slug (lowercase, alphanumerics
/// joined by single hyphens).
fn tag_slug(query: &str) -> String {
    let mut slug = String::new();
    let mut prev_hyphen = false;
    for ch in query.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_hyphen = false;
        } else if !prev_hyphen && !slug.is_empty() {
            slug.push('-');
            prev_hyphen = true;
        }
    }
    slug.trim_end_matches('-').to_string()
}

fn parse(xml: &str, max: usize) -> Vec<SearchResult> {
    let mut out = Vec::new();
    for cap in ITEM_RE.captures_iter(xml) {
        let item = &cap[1];
        let url = LINK_RE
            .captures(item)
            .map(|c| clean(&c[1]))
            .unwrap_or_default();
        if url.is_empty() {
            continue;
        }
        let title = TITLE_RE
            .captures(item)
            .map(|c| clean(&c[1]))
            .unwrap_or_default();
        let snippet = DESC_RE
            .captures(item)
            .map(|c| truncate_chars(&html_to_text(&strip_cdata(&c[1])), 300))
            .unwrap_or_default();
        out.push(SearchResult {
            title,
            url,
            snippet,
            ..Default::default()
        });
        if out.len() >= max {
            break;
        }
    }
    out
}

fn strip_cdata(s: &str) -> String {
    s.trim()
        .trim_start_matches("<![CDATA[")
        .trim_end_matches("]]>")
        .trim()
        .to_string()
}

fn clean(s: &str) -> String {
    decode_entities(strip_cdata(s).trim())
}

#[cfg(test)]
mod tests {
    #[test]
    fn tag_slug_normalizes_query() {
        assert_eq!(super::tag_slug("Rust  Async!"), "rust-async");
        assert_eq!(super::tag_slug("  machine learning  "), "machine-learning");
    }

    #[test]
    fn parses_rss_items() {
        let xml = r#"<rss><channel>
            <item>
              <title><![CDATA[Hello World]]></title>
              <link>https://medium.com/p/abc123</link>
              <description><![CDATA[<p>Body text here</p>]]></description>
            </item>
        </channel></rss>"#;
        let out = super::parse(xml, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://medium.com/p/abc123");
        assert_eq!(out[0].title, "Hello World");
        assert!(out[0].snippet.contains("Body text here"));
    }
}

#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent(crate::LODESTONE_UA)
            .build()
            .unwrap()
    }

    #[tokio::test]
    #[ignore]
    async fn medium_tag_rss_live() {
        let r = http()
            .get("https://medium.com/feed/tag/programming")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let body = r.text().await.unwrap();
        assert!(body.contains("<rss"));
        assert!(body.contains("<item"));
    }
}
