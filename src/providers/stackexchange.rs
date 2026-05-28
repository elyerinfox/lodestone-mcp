//! StackOverflow / StackExchange Q&A provider.
//!
//! One provider, two sourcing modes chosen per call by the model via the
//! `render` flag (the same lever used everywhere else):
//!   * `render = false` (default) → the keyless public API. An optional API key
//!     raises the per-IP quota.
//!   * `render = true` → scrape stackoverflow.com via the shared headless
//!     browser (no quota). Only applies to the `stackoverflow` site; other
//!     StackExchange sites always use the API.

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::decode_entities;

pub(super) struct StackExchange {
    key: String,
}

impl StackExchange {
    pub(super) fn new(key: String) -> Self {
        Self { key }
    }

    async fn search_api(
        &self,
        http: &Client,
        query: &SearchQuery,
        site: &str,
    ) -> Result<Vec<SearchResult>> {
        let pagesize = query.limit.clamp(1, 50).to_string();
        let mut params = vec![
            ("order", "desc"),
            ("sort", "relevance"),
            ("q", query.text.as_str()),
            ("site", site),
            ("pagesize", pagesize.as_str()),
            ("filter", "default"),
        ];
        if !self.key.is_empty() {
            params.push(("key", self.key.as_str()));
        }
        let v: serde_json::Value = http
            .get("https://api.stackexchange.com/2.3/search/advanced")
            .query(&params)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(parse_api(&v, query.limit))
    }

    async fn search_scrape(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        use crate::browser::PageRenderer;
        let url = url::Url::parse_with_params(
            "https://stackoverflow.com/search",
            &[("q", query.text.as_str())],
        )?;
        let html = crate::browser::shared_global().render(url.as_str()).await?;
        let hits = parse_scrape(&html, query.limit);
        if hits.is_empty() && html.to_ascii_lowercase().contains("captcha") {
            return Err(anyhow::anyhow!("StackOverflow served a CAPTCHA page"));
        }
        Ok(hits)
    }
}

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
        if query.render && site == "stackoverflow" {
            return self.search_scrape(query).await;
        }
        self.search_api(http, query, site).await
    }
}

fn parse_api(v: &serde_json::Value, max: usize) -> Vec<SearchResult> {
    let items = match v.get("items").and_then(|i| i.as_array()) {
        Some(i) => i,
        None => return vec![],
    };
    let mut out = Vec::new();
    for q in items {
        let title = decode_entities(
            q.get("title")
                .and_then(|x| x.as_str())
                .unwrap_or("(untitled)"),
        );
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

fn parse_scrape(html: &str, max: usize) -> Vec<SearchResult> {
    use scraper::{Html, Selector};

    use crate::util::collapse_ws;

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
