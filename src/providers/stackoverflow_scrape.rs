//! StackOverflow Q&A provider that scrapes the site via the shared headless
//! browser (feature `browser`) instead of the keyless API — useful to avoid the
//! API's per-IP daily quota. StackOverflow bot-walls plain HTTP search (it
//! redirects to a CAPTCHA), so a real browser is required.
//!
//! Only scrapes `stackoverflow.com`; for other StackExchange sites use the
//! API-backed `stackoverflow` provider.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use scraper::{Html, Selector};
use url::Url;

use crate::browser::PageRenderer;
use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::collapse_ws;

pub(super) struct StackOverflowScrape;

impl StackOverflowScrape {
    pub(super) fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SearchProvider for StackOverflowScrape {
    fn id(&self) -> &'static str {
        "stackoverflow_scrape"
    }
    fn kind(&self) -> ProviderKind {
        ProviderKind::Qa
    }
    async fn search(&self, _http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        // This provider only handles stackoverflow.com.
        if matches!(query.site.as_deref(), Some(site) if site != "stackoverflow") {
            return Ok(vec![]);
        }
        let url = Url::parse_with_params(
            "https://stackoverflow.com/search",
            &[("q", query.text.as_str())],
        )?;
        let html = crate::browser::shared_global().render(url.as_str()).await?;
        let hits = parse(&html, query.limit);
        if hits.is_empty() && html.to_ascii_lowercase().contains("captcha") {
            return Err(anyhow!("StackOverflow served a CAPTCHA page"));
        }
        Ok(hits)
    }
}

fn parse(html: &str, max: usize) -> Vec<SearchResult> {
    let doc = Html::parse_document(html);
    let summary_sel = Selector::parse(".s-post-summary").unwrap();
    let title_sel = Selector::parse("a.s-link").unwrap();
    let excerpt_sel = Selector::parse(".s-post-summary--content-excerpt").unwrap();
    let stat_sel = Selector::parse(".s-post-summary--stats-item").unwrap();
    let accepted_sel = Selector::parse(".s-post-summary--stats-item.has-accepted-answer").unwrap();
    let tag_sel = Selector::parse("a.post-tag").unwrap();

    let mut out = Vec::new();
    for summary in doc.select(&summary_sel) {
        let anchor = match summary.select(&title_sel).next() {
            Some(a) => a,
            None => continue,
        };
        let href = anchor.value().attr("href").unwrap_or("");
        if href.is_empty() {
            continue;
        }
        let url = if href.starts_with("http") {
            href.to_string()
        } else {
            format!("https://stackoverflow.com{href}")
        };
        let title = collapse_ws(&anchor.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        let snippet = summary
            .select(&excerpt_sel)
            .next()
            .map(|e| collapse_ws(&e.text().collect::<String>()))
            .unwrap_or_default();

        let mut score = None;
        let mut answers = 0i64;
        for item in summary.select(&stat_sel) {
            let raw = item
                .value()
                .attr("title")
                .map(|t| t.to_string())
                .unwrap_or_else(|| collapse_ws(&item.text().collect::<String>()));
            let (num, label) = parse_stat(&raw);
            if label.contains("vote") {
                score = Some(num);
            } else if label.contains("answer") {
                answers = num;
            }
        }
        let accepted = summary.select(&accepted_sel).next().is_some();
        let tags: Vec<String> = summary
            .select(&tag_sel)
            .map(|t| collapse_ws(&t.text().collect::<String>()))
            .filter(|t| !t.is_empty())
            .collect();
        let meta = format!(
            "{answers} answers{}{}",
            if accepted { " · accepted ✓" } else { "" },
            if tags.is_empty() {
                String::new()
            } else {
                format!(" · tags: {}", tags.join(", "))
            }
        );

        out.push(SearchResult {
            title,
            url,
            snippet,
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

/// Parse a StackOverflow stat string like "1,234 votes" into `(number, label)`.
fn parse_stat(s: &str) -> (i64, String) {
    let cleaned = s.replace(',', "");
    let mut num = 0i64;
    let mut label = String::new();
    for token in cleaned.split_whitespace() {
        match token.parse::<i64>() {
            Ok(n) => num = n,
            Err(_) => {
                label.push_str(&token.to_ascii_lowercase());
                label.push(' ');
            }
        }
    }
    (num, label)
}
