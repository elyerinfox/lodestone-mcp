//! PubMed skill (keyless): search the biomedical literature and read abstracts via
//! NCBI's E-utilities (the same API `Bio.Entrez` uses). No API key required; if a
//! `LODESTONE_NCBI_API_KEY` is set in the environment it's passed through to raise
//! the rate limit. Results are cached.
//!
//! `pubmed_search` runs `esearch` → `esummary` (PMIDs + citation metadata);
//! `pubmed_summary` adds `efetch` to return a paper's abstract. Links point at
//! https://pubmed.ncbi.nlm.nih.gov/<pmid>/.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, invalid, text_result};

const EUTILS: &str = "https://eutils.ncbi.nlm.nih.gov/entrez/eutils";

/// Percent-encode a query value for the URL.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Common trailing query params: identify the tool, and pass an NCBI API key when
/// `LODESTONE_NCBI_API_KEY` is set (keyless otherwise).
fn common_params() -> String {
    let mut p = String::from("&tool=lodestone-mcp");
    if let Ok(key) = std::env::var("LODESTONE_NCBI_API_KEY") {
        let key = key.trim();
        if !key.is_empty() {
            p.push_str(&format!("&api_key={}", enc(key)));
        }
    }
    p
}

/// `esearch` → the matching PMIDs (most relevant first).
async fn esearch(http: &Client, query: &str, retmax: usize) -> Result<Vec<String>> {
    let url = format!(
        "{EUTILS}/esearch.fcgi?db=pubmed&retmode=json&retmax={retmax}&term={}{}",
        enc(query),
        common_params()
    );
    let v: Value = http
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(v.pointer("/esearchresult/idlist")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

/// `esummary` for a set of PMIDs → the `result` object (per-uid metadata).
async fn esummary(http: &Client, ids: &[String]) -> Result<Value> {
    let url = format!(
        "{EUTILS}/esummary.fcgi?db=pubmed&retmode=json&id={}{}",
        ids.join(","),
        common_params()
    );
    Ok(http
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Author list → "First A, Second B, … et al." (capped).
fn authors_str(doc: &Value) -> String {
    let Some(list) = doc.get("authors").and_then(Value::as_array) else {
        return String::new();
    };
    let names: Vec<&str> = list
        .iter()
        .filter_map(|a| a.get("name").and_then(Value::as_str))
        .collect();
    if names.is_empty() {
        return String::new();
    }
    let shown = names.iter().take(4).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > 4 {
        format!("{shown}, et al.")
    } else {
        shown
    }
}

/// Pull a DOI from `elocationid` (e.g. "doi: 10.x/...") or the `articleids` array.
fn doi_of(doc: &Value) -> Option<String> {
    if let Some(s) = doc.get("elocationid").and_then(Value::as_str) {
        if let Some(i) = s.find("10.") {
            return Some(s[i..].trim().to_string());
        }
    }
    doc.get("articleids")
        .and_then(Value::as_array)?
        .iter()
        .find(|a| a.get("idtype").and_then(Value::as_str) == Some("doi"))
        .and_then(|a| a.get("value").and_then(Value::as_str))
        .map(str::to_string)
}

fn pubmed_url(pmid: &str) -> String {
    format!("https://pubmed.ncbi.nlm.nih.gov/{pmid}/")
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchArgs {
    /// PubMed query — free text or field tags, e.g. `crispr off-target` or
    /// `asthma[Title] AND 2023[Date - Publication]`.
    query: String,
    /// Max results (default 10, capped at 50).
    #[serde(default)]
    max_results: Option<usize>,
}

pub struct PubmedSearch;
impl Skill for PubmedSearch {
    fn name(&self) -> &'static str {
        "pubmed_search"
    }
    fn description(&self) -> &'static str {
        "Search PubMed (biomedical literature) via NCBI E-utilities — keyless. Returns matching \
        papers with PMID, title, authors, journal, date, and a pubmed.ncbi.nlm.nih.gov link. Use \
        pubmed_summary for a paper's abstract. Supports PubMed field tags (e.g. [Title], [Author])."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SearchArgs>()?;
            let query = args.query.trim();
            if query.is_empty() {
                return Err(invalid("empty query"));
            }
            let max = args.max_results.unwrap_or(10).clamp(1, 50);
            let key = format!("pubmed_search|{max}|{query}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let ids = esearch(&server.http, query, max).await.map_err(internal)?;
            if ids.is_empty() {
                return Ok(text_result(format!("No PubMed results for '{query}'.")));
            }
            let sum = esummary(&server.http, &ids).await.map_err(internal)?;
            let result = sum.get("result");
            let mut lines = vec![format!("{} PubMed result(s) for '{query}':", ids.len())];
            for (i, pmid) in ids.iter().enumerate() {
                let Some(doc) = result.and_then(|r| r.get(pmid)) else {
                    continue;
                };
                let title = doc
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("(no title)");
                let journal = doc.get("source").and_then(Value::as_str).unwrap_or("");
                let date = doc.get("pubdate").and_then(Value::as_str).unwrap_or("");
                let authors = authors_str(doc);
                lines.push(format!("\n{}. {title}", i + 1));
                let meta: Vec<&str> = [authors.as_str(), journal, date]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect();
                if !meta.is_empty() {
                    lines.push(format!("   {}", meta.join(" · ")));
                }
                lines.push(format!("   PMID {pmid} — {}", pubmed_url(pmid)));
            }
            let report = lines.join("\n");
            server.retrieval_put(key, &report);
            Ok(text_result(report))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SummaryArgs {
    /// PubMed ID (PMID), e.g. `38000000`.
    pmid: String,
    /// Max characters of abstract text to return (default 3000).
    #[serde(default)]
    max_chars: Option<usize>,
}

pub struct PubmedSummary;
impl Skill for PubmedSummary {
    fn name(&self) -> &'static str {
        "pubmed_summary"
    }
    fn description(&self) -> &'static str {
        "Fetch a PubMed paper's citation + abstract by PMID via NCBI E-utilities (keyless). Returns \
        title, authors, journal/date, DOI, the pubmed.ncbi.nlm.nih.gov link, and the abstract text."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SummaryArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SummaryArgs>()?;
            let pmid = args.pmid.trim();
            if pmid.is_empty() || !pmid.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid("pmid must be a numeric PubMed id"));
            }
            let max_chars = args.max_chars.unwrap_or(3000).clamp(200, 20000);
            let key = format!("pubmed_summary|{max_chars}|{pmid}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let sum = esummary(&server.http, &[pmid.to_string()])
                .await
                .map_err(internal)?;
            let doc = sum
                .pointer(&format!("/result/{pmid}"))
                .filter(|d| d.get("uid").is_some())
                .ok_or_else(|| invalid(format!("no PubMed record for PMID {pmid}")))?;

            let title = doc
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("(no title)");
            let journal = doc.get("source").and_then(Value::as_str).unwrap_or("");
            let date = doc.get("pubdate").and_then(Value::as_str).unwrap_or("");
            let authors = authors_str(doc);
            let mut head = vec![title.to_string()];
            if !authors.is_empty() {
                head.push(authors);
            }
            let mut cite: Vec<String> = Vec::new();
            if !journal.is_empty() || !date.is_empty() {
                cite.push(
                    [journal, date]
                        .iter()
                        .filter(|s| !s.is_empty())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" · "),
                );
            }
            if let Some(doi) = doi_of(doc) {
                cite.push(format!("doi:{doi}"));
            }
            if !cite.is_empty() {
                head.push(cite.join(" · "));
            }
            head.push(pubmed_url(pmid));

            // efetch the abstract as plain text.
            let url = format!(
                "{EUTILS}/efetch.fcgi?db=pubmed&rettype=abstract&retmode=text&id={pmid}{}",
                common_params()
            );
            let abstract_text = server
                .http
                .get(&url)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|e| internal(e.into()))?
                .text()
                .await
                .map_err(|e| internal(e.into()))?;

            let report = format!(
                "{}\n\n{}",
                head.join("\n"),
                truncate_chars(abstract_text.trim(), max_chars)
            );
            server.retrieval_put(key, &report);
            Ok(text_result(report))
        })
    }
}

/// Always-on, keyless (still gateable via `[tools]`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(PubmedSearch), Box::new(PubmedSummary)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_query() {
        assert_eq!(enc("crispr off-target"), "crispr%20off-target");
        assert_eq!(enc("asthma[Title]"), "asthma%5BTitle%5D");
    }

    #[test]
    fn authors_capped_with_et_al() {
        let doc = serde_json::json!({
            "authors": [
                {"name": "A One"}, {"name": "B Two"}, {"name": "C Three"},
                {"name": "D Four"}, {"name": "E Five"}
            ]
        });
        let s = authors_str(&doc);
        assert!(s.ends_with("et al."));
        assert!(s.contains("A One"));
        assert!(!s.contains("E Five"));
    }

    #[test]
    fn doi_from_elocationid() {
        let doc = serde_json::json!({ "elocationid": "doi: 10.18294/sc.2023.4462" });
        assert_eq!(doi_of(&doc).as_deref(), Some("10.18294/sc.2023.4462"));
    }

    #[test]
    fn doi_from_articleids() {
        let doc = serde_json::json!({
            "articleids": [
                {"idtype": "pubmed", "value": "38000000"},
                {"idtype": "doi", "value": "10.1/xyz"}
            ]
        });
        assert_eq!(doi_of(&doc).as_deref(), Some("10.1/xyz"));
    }

    #[test]
    fn pubmed_url_format() {
        assert_eq!(pubmed_url("123"), "https://pubmed.ncbi.nlm.nih.gov/123/");
    }
}
