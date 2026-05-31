//! `standards_search` skill — metadata lookup for published standards via the
//! keyless Crossref API. Covers IEEE, SAE, NIST, ISO, ANSI, IEC, … by querying
//! Crossref and filtering to standards/reports/monographs (so journal noise is
//! dropped), with an optional publisher filter.
//!
//! NOTE: IEEE and SAE standards are copyrighted and **paywalled** — this returns
//! metadata + a `doi.org` link, not full text. NIST publications are free: pair
//! this (or `docs_nist`) with `read_pdf` on the linked PDF for the full document.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{clamp, internal, text_result};

const ENDPOINT: &str = "https://api.crossref.org/works";

/// Substring to match a publisher by a short name. Most SDOs include their
/// acronym in Crossref's publisher string (IEEE, SAE, NIST); a few need mapping.
fn publisher_needle(p: &str) -> String {
    match p.trim().to_ascii_lowercase().as_str() {
        "iso" => "international organization for standardization".to_string(),
        "ansi" => "american national standards institute".to_string(),
        "iec" => "international electrotechnical commission".to_string(),
        other => other.to_string(),
    }
}

async fn search(http: &Client, query: &str, rows: usize) -> Result<Value> {
    let rows = rows.to_string();
    Ok(http
        .get(ENDPOINT)
        .query(&[
            ("query.bibliographic", query),
            ("filter", "type:standard,type:report,type:monograph"),
            ("rows", rows.as_str()),
            ("select", "title,DOI,publisher,published,type,URL"),
        ])
        .header("Accept", "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

fn year(item: &Value) -> Option<i64> {
    item.get("published")
        .and_then(|p| p.get("date-parts"))
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|x| x.as_i64())
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StandardsSearchArgs {
    /// What to look for, e.g. "IEEE 802.11", "SAE J1939", "NIST 800-53",
    /// "ISO 26262", or a topic.
    query: String,
    /// Optional publisher filter: ieee, sae, nist, iso, ansi, iec (or any
    /// substring of the publisher name). Narrows results to that body.
    #[serde(default)]
    publisher: Option<String>,
    /// Maximum number of results. Default 8, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
}

pub struct StandardsSearch;
impl Skill for StandardsSearch {
    fn name(&self) -> &'static str {
        "standards_search"
    }
    fn description(&self) -> &'static str {
        "Search published standards/specifications by title via Crossref (keyless): IEEE, SAE, \
        NIST, ISO, ANSI, IEC, … Optional `publisher` filter (e.g. ieee, sae, nist). Returns title, \
        publisher, type, year, DOI, and a doi.org link. NOTE: IEEE/SAE full text is paywalled (this \
        is metadata only); NIST documents are free — use read_pdf on the linked PDF for those."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StandardsSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<StandardsSearchArgs>()?;
            let limit = clamp(args.max_results, 8, 25);
            let publisher = args
                .publisher
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            // Over-fetch when filtering by publisher, so the post-filter still fills.
            let rows = if publisher.is_some() {
                (limit * 5).min(100)
            } else {
                limit
            };
            let key = format!(
                "standards|{limit}|{}|{}",
                publisher.unwrap_or(""),
                args.query
            );
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }

            let v = search(&server.http, &args.query, rows)
                .await
                .map_err(internal)?;
            let empty = Vec::new();
            let items = v
                .pointer("/message/items")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);

            let needle = publisher.map(publisher_needle);
            let mut out = format!("Standards matching \"{}\"", args.query);
            if let Some(p) = publisher {
                out.push_str(&format!(" [{p}]"));
            }
            out.push_str(":\n");

            let mut shown = 0usize;
            for item in items {
                let pubr = item.get("publisher").and_then(|x| x.as_str()).unwrap_or("");
                if let Some(n) = &needle {
                    if !pubr.to_ascii_lowercase().contains(n.as_str()) {
                        continue;
                    }
                }
                let title = item
                    .get("title")
                    .and_then(|x| x.as_array())
                    .and_then(|a| a.first())
                    .and_then(|x| x.as_str())
                    .unwrap_or("(untitled)");
                let kind = item.get("type").and_then(|x| x.as_str()).unwrap_or("");
                let doi = item.get("DOI").and_then(|x| x.as_str()).unwrap_or("");
                let url = item
                    .get("URL")
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("https://doi.org/{doi}"));
                shown += 1;
                out.push_str(&format!("\n{shown}. {title}\n   {pubr}"));
                if !kind.is_empty() {
                    out.push_str(&format!(" · {kind}"));
                }
                if let Some(y) = year(item) {
                    out.push_str(&format!(" · {y}"));
                }
                out.push_str(&format!("\n   DOI {doi}   {url}\n"));
                if shown >= limit {
                    break;
                }
            }
            if shown == 0 {
                return Ok(text_result(format!("No standards match: {}", args.query)));
            }
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(StandardsSearch)]
}

#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        crate::skills::live_http()
    }

    /// Crossref REST — keyless; the politely-mailto'd UA is recommended.
    #[tokio::test]
    #[ignore]
    async fn crossref_standards_search_live() {
        let r = http()
            .get("https://api.crossref.org/works?query.bibliographic=IEEE%20802.11&rows=3")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        assert_eq!(v["status"].as_str(), Some("ok"));
        let items = v["message"]["items"].as_array().expect("missing items");
        assert!(!items.is_empty());
        for k in ["DOI", "title", "type"] {
            assert!(items[0].get(k).is_some(), "missing field {k}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{publisher_needle, year};

    #[test]
    fn publisher_aliases() {
        assert_eq!(publisher_needle("IEEE"), "ieee");
        assert_eq!(publisher_needle("nist"), "nist");
        assert!(publisher_needle("iso").contains("international organization"));
    }

    #[test]
    fn extracts_year() {
        let v = serde_json::json!({"published": {"date-parts": [[2023, 12]]}});
        assert_eq!(year(&v), Some(2023));
        assert_eq!(year(&serde_json::json!({})), None);
    }
}
