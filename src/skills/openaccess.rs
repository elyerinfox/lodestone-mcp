//! Open-access scholarly lookup (keyless): find **legal** full-text copies of papers.
//!
//! - `unpaywall_lookup` — Unpaywall: given a DOI, return the best open-access copy
//!   (author manuscript, repository deposit, or publisher OA) and all known OA
//!   locations, with a PDF URL you can hand to `read_pdf`.
//! - `openalex_search` / `openalex_work` — OpenAlex: search the scholarly graph or
//!   fetch one work, with OA status and the open-access PDF link when one exists.
//!
//! Both surface only legitimately open-access copies. Unpaywall requires a contact
//! email (its terms) — set `LODESTONE_CONTACT_EMAIL`; OpenAlex uses it for the polite
//! pool when present but works without. Results are cached.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

/// Contact email for Unpaywall (required) and the OpenAlex polite pool (optional).
fn contact_email() -> Option<String> {
    std::env::var("LODESTONE_CONTACT_EMAIL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Strip a DOI down to the bare `10.x/...` form (no `https://doi.org/` or `doi:`).
fn bare_doi(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix("https://doi.org/")
        .or_else(|| s.strip_prefix("http://doi.org/"))
        .or_else(|| s.strip_prefix("https://dx.doi.org/"))
        .or_else(|| s.strip_prefix("doi:"))
        .unwrap_or(s);
    s.trim().to_string()
}

/// Percent-encode a query value.
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

async fn get_json(http: &Client, url: &str) -> Result<Value> {
    Ok(http
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Render one OA location line (`host • version • license → pdf/landing`).
fn oa_location_line(loc: &Value) -> Option<String> {
    let url = loc
        .get("url_for_pdf")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| loc.get("url").and_then(Value::as_str))?;
    let host = loc.get("host_type").and_then(Value::as_str).unwrap_or("");
    let version = loc.get("version").and_then(Value::as_str).unwrap_or("");
    let license = loc.get("license").and_then(Value::as_str).unwrap_or("");
    let tags: Vec<&str> = [host, version, license]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    Some(if tags.is_empty() {
        format!("  {url}")
    } else {
        format!("  [{}] {url}", tags.join(" · "))
    })
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DoiArgs {
    /// A DOI — bare (`10.1038/nature12373`), `doi:…`, or a `https://doi.org/…` URL.
    doi: String,
}

pub struct UnpaywallLookup;
impl Skill for UnpaywallLookup {
    fn name(&self) -> &'static str {
        "unpaywall_lookup"
    }
    fn description(&self) -> &'static str {
        "Find a LEGAL open-access copy of a paper by DOI via Unpaywall (keyless). Returns whether \
        it's open access, the best OA PDF URL (feed it to read_pdf), and all known OA locations \
        (publisher OA, author manuscript, repository deposit). Requires a contact email in \
        LODESTONE_CONTACT_EMAIL (Unpaywall's terms)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DoiArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DoiArgs>()?;
            let doi = bare_doi(&args.doi);
            if doi.is_empty() {
                return Err(invalid("empty DOI"));
            }
            let email = contact_email().ok_or_else(|| {
                invalid(
                    "Unpaywall requires a contact email — set LODESTONE_CONTACT_EMAIL to a real \
                     address (its API terms; example.com is rejected).",
                )
            })?;
            let key = format!("unpaywall|{doi}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let url = format!("https://api.unpaywall.org/v2/{doi}?email={}", enc(&email));
            let v = get_json(&server.http, &url).await.map_err(internal)?;
            if v.get("error").and_then(Value::as_bool) == Some(true) {
                let msg = v.get("message").and_then(Value::as_str).unwrap_or("error");
                return Err(invalid(format!("Unpaywall: {msg}")));
            }

            let title = v
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("(untitled)");
            let year = v.get("year").and_then(Value::as_i64);
            let journal = v.get("journal_name").and_then(Value::as_str).unwrap_or("");
            let is_oa = v.get("is_oa").and_then(Value::as_bool).unwrap_or(false);
            let status = v.get("oa_status").and_then(Value::as_str).unwrap_or("");

            let mut lines = vec![format!("{title}")];
            let mut meta: Vec<String> = vec![format!("doi:{doi}")];
            if let Some(y) = year {
                meta.push(y.to_string());
            }
            if !journal.is_empty() {
                meta.push(journal.to_string());
            }
            lines.push(format!("  {}", meta.join(" · ")));
            lines.push(format!(
                "  open access: {}{}",
                if is_oa { "yes" } else { "no" },
                if status.is_empty() {
                    String::new()
                } else {
                    format!(" ({status})")
                }
            ));
            if let Some(best) = v.get("best_oa_location").filter(|b| !b.is_null()) {
                if let Some(l) = oa_location_line(best) {
                    lines.push(format!("  best OA →{}", l.trim_start()));
                }
            }
            if let Some(locs) = v.get("oa_locations").and_then(Value::as_array) {
                if locs.len() > 1 {
                    lines.push(format!("  all {} OA location(s):", locs.len()));
                    for loc in locs {
                        if let Some(l) = oa_location_line(loc) {
                            lines.push(l);
                        }
                    }
                }
            }
            if !is_oa {
                lines.push(
                    "  No open-access copy found. Try openalex_search, arxiv_search, or \
                     ncbi_search db=pmc."
                        .to_string(),
                );
            }
            let report = lines.join("\n");
            server.retrieval_put(key, &report);
            Ok(text_result(report))
        })
    }
}

/// Author list from an OpenAlex work → "A, B, … et al." (capped).
fn openalex_authors(work: &Value) -> String {
    let Some(list) = work.get("authorships").and_then(Value::as_array) else {
        return String::new();
    };
    let names: Vec<&str> = list
        .iter()
        .filter_map(|a| a.pointer("/author/display_name").and_then(Value::as_str))
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

/// One-work summary lines from an OpenAlex work object.
fn openalex_work_lines(work: &Value) -> Vec<String> {
    let title = work
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("(untitled)");
    let year = work.get("publication_year").and_then(Value::as_i64);
    let doi = work
        .get("doi")
        .and_then(Value::as_str)
        .map(|d| d.trim_start_matches("https://doi.org/").to_string());
    let venue = work
        .pointer("/primary_location/source/display_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let oa = work.get("open_access");
    let is_oa = oa
        .and_then(|o| o.get("is_oa"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let oa_pdf = work
        .pointer("/best_oa_location/pdf_url")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| oa.and_then(|o| o.get("oa_url")).and_then(Value::as_str));

    let mut lines = vec![title.to_string()];
    let authors = openalex_authors(work);
    if !authors.is_empty() {
        lines.push(format!("  {authors}"));
    }
    let mut meta: Vec<String> = Vec::new();
    if let Some(y) = year {
        meta.push(y.to_string());
    }
    if !venue.is_empty() {
        meta.push(venue.to_string());
    }
    if let Some(d) = &doi {
        meta.push(format!("doi:{d}"));
    }
    if !meta.is_empty() {
        lines.push(format!("  {}", meta.join(" · ")));
    }
    match oa_pdf {
        Some(pdf) if is_oa => lines.push(format!("  OA PDF → {pdf}")),
        _ => lines.push(format!(
            "  open access: {}",
            if is_oa { "yes" } else { "no" }
        )),
    }
    lines
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct OpenAlexSearchArgs {
    /// Free-text search over titles/abstracts/metadata.
    query: String,
    /// Max results (default 10, capped at 50).
    #[serde(default)]
    max_results: Option<usize>,
}

pub struct OpenAlexSearch;
impl Skill for OpenAlexSearch {
    fn name(&self) -> &'static str {
        "openalex_search"
    }
    fn description(&self) -> &'static str {
        "Search the OpenAlex scholarly graph (keyless): papers with authors, year, venue, DOI, and \
        the open-access PDF link when one exists (feed it to read_pdf). Use openalex_work or \
        unpaywall_lookup to resolve a specific DOI."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<OpenAlexSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<OpenAlexSearchArgs>()?;
            let query = args.query.trim();
            if query.is_empty() {
                return Err(invalid("empty query"));
            }
            let max = args.max_results.unwrap_or(10).clamp(1, 50);
            let key = format!("openalex_search|{max}|{query}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let mut url = format!(
                "https://api.openalex.org/works?search={}&per-page={max}",
                enc(query)
            );
            if let Some(email) = contact_email() {
                url.push_str(&format!("&mailto={}", enc(&email)));
            }
            let v = get_json(&server.http, &url).await.map_err(internal)?;
            let results = v.get("results").and_then(Value::as_array);
            let Some(results) = results.filter(|r| !r.is_empty()) else {
                return Ok(text_result(format!("No OpenAlex results for '{query}'.")));
            };
            let mut lines = vec![format!(
                "{} OpenAlex result(s) for '{query}':",
                results.len()
            )];
            for (i, w) in results.iter().enumerate() {
                lines.push(format!("\n{}.", i + 1));
                lines.extend(openalex_work_lines(w));
            }
            let report = lines.join("\n");
            server.retrieval_put(key, &report);
            Ok(text_result(report))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct OpenAlexWorkArgs {
    /// A DOI (`10.…`, `doi:…`, or a doi.org URL) or an OpenAlex work id (`W…`).
    id: String,
}

pub struct OpenAlexWork;
impl Skill for OpenAlexWork {
    fn name(&self) -> &'static str {
        "openalex_work"
    }
    fn description(&self) -> &'static str {
        "Fetch one work from OpenAlex by DOI or OpenAlex id (keyless): title, authors, year, venue, \
        DOI, open-access status and PDF link. Pair with unpaywall_lookup for the fullest OA picture."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<OpenAlexWorkArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<OpenAlexWorkArgs>()?;
            let raw = args.id.trim();
            if raw.is_empty() {
                return Err(invalid("empty id"));
            }
            // DOI-shaped input → `doi:<bare>`; otherwise an OpenAlex id (e.g. W123…).
            let selector =
                if raw.starts_with("10.") || raw.contains("doi.org/") || raw.starts_with("doi:") {
                    format!("doi:{}", bare_doi(raw))
                } else {
                    raw.trim_start_matches("https://openalex.org/").to_string()
                };
            let key = format!("openalex_work|{selector}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let mut url = format!("https://api.openalex.org/works/{}", enc(&selector));
            if let Some(email) = contact_email() {
                url.push_str(&format!("?mailto={}", enc(&email)));
            }
            let v = get_json(&server.http, &url)
                .await
                .map_err(|_| invalid(format!("no OpenAlex work for '{raw}'")))?;
            if v.get("id").and_then(Value::as_str).is_none() {
                return Err(invalid(format!("no OpenAlex work for '{raw}'")));
            }
            let report = openalex_work_lines(&v).join("\n");
            server.retrieval_put(key, &report);
            Ok(text_result(report))
        })
    }
}

/// Always-on, keyless (still gateable via `[tools]`).
#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent("lodestone-mcp/0.1.0 (+https://github.com/elyerinfox/lodestone-mcp)")
            .build()
            .unwrap()
    }

    /// Unpaywall rejects example.com emails — needs LODESTONE_CONTACT_EMAIL.
    #[tokio::test]
    #[ignore]
    async fn unpaywall_live() {
        let email = match std::env::var("LODESTONE_CONTACT_EMAIL") {
            Ok(e) if !e.trim().is_empty() => e,
            _ => {
                eprintln!("skipping unpaywall live: no LODESTONE_CONTACT_EMAIL");
                return;
            }
        };
        // 10.1038/s41586-020-2649-2 = the Nature 2020 paper on Array Programming (NumPy).
        let url = format!("https://api.unpaywall.org/v2/10.1038/s41586-020-2649-2?email={email}");
        let r = http().get(&url).send().await.expect("network").error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        for k in ["doi", "title", "is_oa"] {
            assert!(v.get(k).is_some(), "missing field {k}");
        }
    }

    /// OpenAlex is keyless but recommends a polite mailto in the User-Agent.
    #[tokio::test]
    #[ignore]
    async fn openalex_search_live() {
        let r = http()
            .get("https://api.openalex.org/works?search=transformer+attention&per_page=3")
            .send().await.expect("network").error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        let results = v["results"].as_array().expect("missing results");
        assert!(!results.is_empty());
        for k in ["id", "title", "doi"] {
            assert!(results[0].get(k).is_some(), "missing field {k}");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn openalex_work_live() {
        let r = http()
            .get("https://api.openalex.org/works/doi:10.1038/s41586-020-2649-2")
            .send().await.expect("network").error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        assert!(v.get("title").is_some());
        assert!(v.get("publication_year").is_some());
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(UnpaywallLookup),
        Box::new(OpenAlexSearch),
        Box::new(OpenAlexWork),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_doi_strips_prefixes() {
        assert_eq!(bare_doi("https://doi.org/10.1/x"), "10.1/x");
        assert_eq!(bare_doi("doi:10.2/y"), "10.2/y");
        assert_eq!(bare_doi("  10.3/z  "), "10.3/z");
    }

    #[test]
    fn oa_location_line_prefers_pdf_and_tags() {
        let loc = serde_json::json!({
            "url": "https://x/landing", "url_for_pdf": "https://x/paper.pdf",
            "host_type": "repository", "version": "publishedVersion", "license": "cc-by"
        });
        let l = oa_location_line(&loc).unwrap();
        assert!(l.contains("paper.pdf"));
        assert!(l.contains("repository"));
        assert!(l.contains("cc-by"));
    }

    #[test]
    fn openalex_lines_render_authors_and_oa() {
        let w = serde_json::json!({
            "title": "A Study",
            "publication_year": 2021,
            "doi": "https://doi.org/10.1/abc",
            "primary_location": {"source": {"display_name": "Nature"}},
            "open_access": {"is_oa": true, "oa_url": "https://x/p.pdf"},
            "best_oa_location": {"pdf_url": "https://x/best.pdf"},
            "authorships": [{"author": {"display_name": "Jane Doe"}}]
        });
        let out = openalex_work_lines(&w).join("\n");
        assert!(out.contains("A Study"));
        assert!(out.contains("Jane Doe"));
        assert!(out.contains("doi:10.1/abc"));
        assert!(out.contains("best.pdf"));
    }

    #[test]
    fn enc_escapes() {
        assert_eq!(enc("a b/c"), "a%20b%2Fc");
    }
}
