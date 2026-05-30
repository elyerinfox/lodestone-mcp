//! Wikipedia skills (keyless): `wikipedia_search` (full-text search via the
//! MediaWiki API) and `wikipedia_summary` (a page's lead extract via the REST
//! API, or the full plain-text article with `full=true`). Language is
//! configurable (`lang`, default `en`). No account or key.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::{collapse_ws, html_to_text, truncate_chars};
use crate::{clamp, internal, text_result};

/// Sanitize a language code to a safe subdomain (lowercase alnum + `-`), default `en`.
fn lang_code(input: Option<&str>) -> String {
    let raw = input
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("en");
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    if cleaned.is_empty() {
        "en".to_string()
    } else {
        cleaned.to_ascii_lowercase()
    }
}

/// Percent-encode a Wikipedia title for a REST path segment (spaces → `_`).
fn enc_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for b in title.trim().replace(' ', "_").bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn article_url(lang: &str, title: &str) -> String {
    format!("https://{lang}.wikipedia.org/wiki/{}", enc_title(title))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WikipediaSearchArgs {
    /// What to search Wikipedia for.
    query: String,
    /// Wikipedia language edition (e.g. "en", "de", "ja"). Default "en".
    #[serde(default)]
    lang: Option<String>,
    /// Maximum number of results. Default 8, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WikipediaSummaryArgs {
    /// The article title, e.g. "Linux" or "Rust (programming language)".
    title: String,
    /// Wikipedia language edition (e.g. "en", "de", "ja"). Default "en".
    #[serde(default)]
    lang: Option<String>,
    /// Return the full plain-text article instead of just the lead summary.
    #[serde(default)]
    full: Option<bool>,
    /// Max characters to return (for `full`). Omit for the server default.
    #[serde(default)]
    max_chars: Option<u32>,
}

pub struct WikipediaSearch;
impl Skill for WikipediaSearch {
    fn name(&self) -> &'static str {
        "wikipedia_search"
    }
    fn description(&self) -> &'static str {
        "Search Wikipedia (keyless) via the MediaWiki API. Returns matching article titles, a \
        snippet, and the URL. `lang` selects the edition (default en). Use wikipedia_summary to \
        read one."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WikipediaSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<WikipediaSearchArgs>()?;
            let lang = lang_code(args.lang.as_deref());
            let limit = clamp(args.max_results, 8, 25);
            let srlimit = limit.to_string();
            let url = format!("https://{lang}.wikipedia.org/w/api.php");
            let v: Value = server
                .http
                .get(&url)
                .query(&[
                    ("action", "query"),
                    ("list", "search"),
                    ("srsearch", args.query.as_str()),
                    ("srlimit", srlimit.as_str()),
                    ("format", "json"),
                ])
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| internal(e.into()))?
                .error_for_status()
                .map_err(|e| internal(e.into()))?
                .json()
                .await
                .map_err(|e| internal(e.into()))?;

            let empty = Vec::new();
            let results = v
                .pointer("/query/search")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            if results.is_empty() {
                return Ok(text_result(format!(
                    "No Wikipedia ({lang}) results for: {}",
                    args.query
                )));
            }
            let mut out = format!("Wikipedia ({lang}) results for \"{}\":\n", args.query);
            for r in results.iter().take(limit) {
                let title = r.get("title").and_then(|x| x.as_str()).unwrap_or("");
                let snippet = r
                    .get("snippet")
                    .and_then(|x| x.as_str())
                    .map(|s| collapse_ws(&html_to_text(s)))
                    .unwrap_or_default();
                out.push_str(&format!("\n{title}\n   {}\n", article_url(&lang, title)));
                if !snippet.is_empty() {
                    out.push_str(&format!("   {snippet}\n"));
                }
            }
            Ok(text_result(out))
        })
    }
}

pub struct WikipediaSummary;
impl Skill for WikipediaSummary {
    fn name(&self) -> &'static str {
        "wikipedia_summary"
    }
    fn description(&self) -> &'static str {
        "Read a Wikipedia article (keyless): the lead summary by default, or the full plain-text \
        article with full=true. Accepts an article title; `lang` selects the edition (default en)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WikipediaSummaryArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<WikipediaSummaryArgs>()?;
            let lang = lang_code(args.lang.as_deref());
            let full = args.full.unwrap_or(false);
            let max = server.clamp_chars(args.max_chars);
            let title = args.title.trim();
            let key = format!("wiki|{lang}|{full}|{max}|{title}");
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }

            let out = if full {
                let url = format!("https://{lang}.wikipedia.org/w/api.php");
                let v: Value = server
                    .http
                    .get(&url)
                    .query(&[
                        ("action", "query"),
                        ("prop", "extracts"),
                        ("explaintext", "1"),
                        ("redirects", "1"),
                        ("titles", title),
                        ("format", "json"),
                    ])
                    .header("Accept", "application/json")
                    .send()
                    .await
                    .map_err(|e| internal(e.into()))?
                    .error_for_status()
                    .map_err(|e| internal(e.into()))?
                    .json()
                    .await
                    .map_err(|e| internal(e.into()))?;
                let extract = v
                    .pointer("/query/pages")
                    .and_then(|p| p.as_object())
                    .and_then(|m| m.values().next())
                    .and_then(|page| page.get("extract"))
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty());
                match extract {
                    Some(text) => format!(
                        "{title} — {}\n\n{}",
                        article_url(&lang, title),
                        truncate_chars(text, max)
                    ),
                    None => {
                        return Ok(text_result(format!(
                            "No Wikipedia ({lang}) article: {title}"
                        )))
                    }
                }
            } else {
                let url = format!(
                    "https://{lang}.wikipedia.org/api/rest_v1/page/summary/{}",
                    enc_title(title)
                );
                let resp = server
                    .http
                    .get(&url)
                    .header("Accept", "application/json")
                    .send()
                    .await
                    .map_err(|e| internal(e.into()))?;
                if resp.status() == reqwest::StatusCode::NOT_FOUND {
                    return Ok(text_result(format!(
                        "No Wikipedia ({lang}) article: {title}"
                    )));
                }
                let v: Value = resp
                    .error_for_status()
                    .map_err(|e| internal(e.into()))?
                    .json()
                    .await
                    .map_err(|e| internal(e.into()))?;
                let page_title = v.get("title").and_then(|x| x.as_str()).unwrap_or(title);
                let extract = v.get("extract").and_then(|x| x.as_str()).unwrap_or("");
                let link = v
                    .pointer("/content_urls/desktop/page")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| article_url(&lang, page_title));
                format!("{page_title} — {link}\n\n{}", truncate_chars(extract, max))
            };

            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(WikipediaSearch), Box::new(WikipediaSummary)]
}

#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent("lodestone-mcp/0.1.0 (+https://github.com/elyerinfox/lodestone-mcp)")
            .build()
            .unwrap()
    }

    #[tokio::test]
    #[ignore]
    async fn wikipedia_search_live() {
        let r = http()
            .get("https://en.wikipedia.org/w/api.php?action=query&list=search&srsearch=Rust+programming+language&format=json&utf8=1&srlimit=3")
            .send().await.expect("network").error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        let items = v["query"]["search"].as_array().expect("missing search array");
        assert!(!items.is_empty());
        for k in ["title", "pageid", "snippet"] {
            assert!(items[0].get(k).is_some(), "missing field {k}");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn wikipedia_summary_live() {
        // MediaWiki extract API on a stable page.
        let r = http()
            .get("https://en.wikipedia.org/w/api.php?action=query&prop=extracts&exintro=1&explaintext=1&titles=Rust_(programming_language)&format=json&utf8=1")
            .send().await.expect("network").error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        let pages = v["query"]["pages"].as_object().expect("missing pages");
        let first = pages.values().next().expect("no page");
        assert!(first.get("extract").is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::{article_url, enc_title, lang_code};

    #[test]
    fn lang_defaults_and_sanitizes() {
        assert_eq!(lang_code(None), "en");
        assert_eq!(lang_code(Some("DE")), "de");
        assert_eq!(lang_code(Some("zh-yue")), "zh-yue");
        assert_eq!(lang_code(Some("../evil")), "evil");
    }

    #[test]
    fn encodes_titles() {
        assert_eq!(
            enc_title("Rust (programming language)"),
            "Rust_%28programming_language%29"
        );
        assert_eq!(
            article_url("en", "Linux"),
            "https://en.wikipedia.org/wiki/Linux"
        );
    }
}
