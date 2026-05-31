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

pub(crate) struct StackExchange {
    key: String,
}

impl StackExchange {
    pub(crate) fn new(key: String) -> Self {
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

#[cfg(test)]
mod tests {
    #[test]
    fn parse_api_extracts_questions_and_meta() {
        let v = serde_json::json!({
            "items": [{
                "title": "How to &amp; why",
                "link": "https://stackoverflow.com/q/1",
                "score": 42,
                "answer_count": 3,
                "accepted_answer_id": 99,
                "tags": ["rust", "async"]
            }]
        });
        let out = super::parse_api(&v, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "How to & why"); // HTML entities decoded
        assert_eq!(out[0].url, "https://stackoverflow.com/q/1");
        assert_eq!(out[0].score, Some(42));
        let meta = out[0].meta.as_deref().unwrap();
        assert!(meta.contains("3 answers"), "meta was {meta:?}");
        assert!(meta.contains("accepted"), "meta was {meta:?}");
        assert!(meta.contains("rust"), "meta was {meta:?}");
    }

    #[test]
    fn parse_stat_splits_number_and_label() {
        assert_eq!(
            super::parse_stat("1,234 votes"),
            (1234, "votes ".to_string())
        );
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
    async fn stackexchange_search_live() {
        let key = std::env::var("LODESTONE_STACKEXCHANGE_KEY").unwrap_or_default();
        let key_q = if key.is_empty() {
            String::new()
        } else {
            format!("&key={key}")
        };
        let url = format!("https://api.stackexchange.com/2.3/search?order=desc&sort=relevance&intitle=rust&site=stackoverflow&pagesize=3{key_q}");
        let r = http().get(&url).send().await.expect("network");
        if matches!(r.status().as_u16(), 429 | 503) {
            eprintln!("stackexchange rate-limited: {}", r.status());
            return;
        }
        let r = r.error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        let items = v["items"].as_array().expect("missing items");
        assert!(!items.is_empty());
        for k in ["title", "link", "score", "answer_count"] {
            assert!(items[0].get(k).is_some(), "missing field {k}");
        }
    }
}
