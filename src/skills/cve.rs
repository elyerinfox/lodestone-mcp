//! CVE / CPE lookup skills (network): query the NIST National Vulnerability
//! Database (NVD) by CVE id, keyword, or CPE 2.3 id. Keyless — NVD's public
//! API is free; an optional key just raises the rate limit and is not
//! configured here yet. LLMs hallucinate CVE descriptions and CVSS scores
//! reliably, including for CVEs that don't exist. The right answer is to
//! ask NVD.
//!
//! ## Sources
//!
//! - [NIST NVD CVE API v2.0](https://nvd.nist.gov/developers/vulnerabilities).
//! - CVSS v3.1 specification (FIRST.org).

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{internal, invalid, text_result};

const CVE_API: &str = "https://services.nvd.nist.gov/rest/json/cves/2.0";
const CPE_API: &str = "https://services.nvd.nist.gov/rest/json/cpes/2.0";

fn normalize_cve(id: &str) -> Result<String, McpError> {
    let up = id.trim().to_ascii_uppercase();
    if !up.starts_with("CVE-") || up.len() < 9 {
        return Err(invalid(format!(
            "`{id}` doesn't look like a CVE id (expected `CVE-YYYY-NNNN`)"
        )));
    }
    Ok(up)
}

// ---------------------------------------------------------------------------
// cve_get
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetArgs {
    /// CVE id, e.g. `CVE-2024-12345`. Case-insensitive.
    cve_id: String,
}

pub struct CveGet;
impl Skill for CveGet {
    fn name(&self) -> &'static str {
        "cve_get"
    }
    fn description(&self) -> &'static str {
        "Fetch one CVE record from NIST NVD by id. Returns description, CVSS v3.1 vector + base \
         score + severity, CWE, affected CPE list, references. Keyless. Returns a clear \
         `not_found` if NVD has no record for that id (LLMs frequently fabricate IDs)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GetArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<GetArgs>()?;
            let cve_id = normalize_cve(&args.cve_id)?;
            let key = format!("cve_get|{cve_id}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let url = format!("{CVE_API}?cveId={cve_id}");
            let v: Value = server
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| internal(e.into()))?
                .error_for_status()
                .map_err(|e| internal(e.into()))?
                .json()
                .await
                .map_err(|e| internal(e.into()))?;
            let total = v.get("totalResults").and_then(|x| x.as_u64()).unwrap_or(0);
            if total == 0 {
                let body = json!({
                    "cve_id": cve_id,
                    "found": false,
                    "note": "NVD has no record for this id. LLMs frequently fabricate CVE numbers — double-check the source that produced this id.",
                })
                .to_string();
                server.retrieval_put(key, &body);
                return Ok(text_result(body));
            }
            let item = v["vulnerabilities"][0]["cve"].clone();
            let body = summarize_cve(&cve_id, &item).to_string();
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "A well-known recent CVE",
                args: r#"{"cve_id": "CVE-2021-44228"}"#,
                note: Some("Log4Shell — the LLM's training likely remembers some details but not the CVSS vector or the full CPE list."),
            },
            SkillExample {
                title: "Probably-fabricated id",
                args: r#"{"cve_id": "CVE-2099-99999"}"#,
                note: Some("Returns found=false with a warning — useful when an LLM cites a CVE you can't verify."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Verify a CVE id an LLM cited before believing the description / score.",
            "Get the canonical NVD description + CVSS vector for a known CVE.",
            "Pull the affected CPE list to know which versions are vulnerable.",
        ]
    }
}

fn summarize_cve(id: &str, cve: &Value) -> Value {
    let description = cve["descriptions"]
        .as_array()
        .and_then(|arr| arr.iter().find(|d| d["lang"] == "en"))
        .and_then(|d| d["value"].as_str())
        .unwrap_or("")
        .to_string();
    let published = cve["published"].as_str().unwrap_or("").to_string();
    let last_modified = cve["lastModified"].as_str().unwrap_or("").to_string();
    let mut cvss_v31_score: Option<f64> = None;
    let mut cvss_v31_vector: Option<String> = None;
    let mut cvss_v31_severity: Option<String> = None;
    if let Some(metrics) = cve["metrics"]["cvssMetricV31"].as_array() {
        if let Some(first) = metrics.first() {
            cvss_v31_score = first["cvssData"]["baseScore"].as_f64();
            cvss_v31_vector = first["cvssData"]["vectorString"]
                .as_str()
                .map(str::to_string);
            cvss_v31_severity = first["cvssData"]["baseSeverity"]
                .as_str()
                .map(str::to_string);
        }
    }
    let cwes: Vec<String> = cve["weaknesses"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|w| {
                    w["description"]
                        .as_array()?
                        .iter()
                        .find(|d| d["lang"] == "en")?["value"]
                        .as_str()
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    let references: Vec<String> = cve["references"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|r| r["url"].as_str().map(str::to_string))
                .take(10)
                .collect()
        })
        .unwrap_or_default();
    let mut affected_cpes: Vec<String> = Vec::new();
    if let Some(configs) = cve["configurations"].as_array() {
        for cfg in configs {
            if let Some(nodes) = cfg["nodes"].as_array() {
                for node in nodes {
                    if let Some(matches) = node["cpeMatch"].as_array() {
                        for m in matches.iter().take(20) {
                            if let Some(cpe) = m["criteria"].as_str() {
                                affected_cpes.push(cpe.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    json!({
        "cve_id": id,
        "found": true,
        "description": description,
        "published": published,
        "last_modified": last_modified,
        "cvss_v31_score": cvss_v31_score,
        "cvss_v31_severity": cvss_v31_severity,
        "cvss_v31_vector": cvss_v31_vector,
        "cwes": cwes,
        "affected_cpes_first_20": affected_cpes,
        "references_first_10": references,
    })
}

// ---------------------------------------------------------------------------
// cve_search
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchArgs {
    /// Free-text keyword to search description fields (e.g. `nginx`, `log4j`).
    #[serde(default)]
    keyword: Option<String>,
    /// Optional CPE 2.3 id to filter by (e.g. `cpe:2.3:a:nginx:nginx:1.27:*:*:*:*:*:*:*`).
    /// Use `cpe_search` to discover the right CPE id.
    #[serde(default)]
    cpe: Option<String>,
    /// ISO date (`YYYY-MM-DD`) lower bound on `pubStartDate`.
    #[serde(default)]
    published_after: Option<String>,
    /// CVSS v3.1 base-score floor (0-10).
    #[serde(default)]
    cvss_v3_min: Option<f64>,
    /// Max rows returned. Defaults to 20; capped at 50.
    #[serde(default)]
    limit: Option<u32>,
}

pub struct CveSearch;
impl Skill for CveSearch {
    fn name(&self) -> &'static str {
        "cve_search"
    }
    fn description(&self) -> &'static str {
        "Search NIST NVD by keyword, CPE id, publication-date floor, and CVSS v3.1 base-score \
         floor. Returns up to `limit` (default 20, max 50) summaries — id, description first \
         line, CVSS score + severity, published date. Use `cve_get` to drill into one record. \
         Keyless."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SearchArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SearchArgs>()?;
            let limit = args.limit.unwrap_or(20).min(50);
            let mut params: Vec<(String, String)> = Vec::new();
            if let Some(k) = args.keyword.as_deref() {
                if !k.trim().is_empty() {
                    params.push(("keywordSearch".into(), k.trim().to_string()));
                }
            }
            if let Some(c) = args.cpe.as_deref() {
                if !c.trim().is_empty() {
                    params.push(("cpeName".into(), c.trim().to_string()));
                }
            }
            if let Some(d) = args.published_after.as_deref() {
                let trimmed = d.trim();
                if !trimmed.is_empty() {
                    params.push(("pubStartDate".into(), format!("{trimmed}T00:00:00.000")));
                }
            }
            if let Some(min) = args.cvss_v3_min {
                params.push(("cvssV3Severity".into(), severity_for(min)));
            }
            params.push(("resultsPerPage".into(), limit.to_string()));
            let key = format!("cve_search|{params:?}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let v: Value = server
                .http
                .get(CVE_API)
                .query(&params)
                .send()
                .await
                .map_err(|e| internal(e.into()))?
                .error_for_status()
                .map_err(|e| internal(e.into()))?
                .json()
                .await
                .map_err(|e| internal(e.into()))?;
            let total = v.get("totalResults").and_then(|x| x.as_u64()).unwrap_or(0);
            let rows: Vec<Value> = v["vulnerabilities"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let cve = item.get("cve")?;
                            let id = cve["id"].as_str()?.to_string();
                            let desc = cve["descriptions"]
                                .as_array()?
                                .iter()
                                .find(|d| d["lang"] == "en")?["value"]
                                .as_str()?
                                .lines()
                                .next()?
                                .to_string();
                            let score = cve["metrics"]["cvssMetricV31"][0]["cvssData"]["baseScore"]
                                .as_f64();
                            let severity = cve["metrics"]["cvssMetricV31"][0]["cvssData"]
                                ["baseSeverity"]
                                .as_str()
                                .map(str::to_string);
                            let published = cve["published"].as_str().map(str::to_string);
                            Some(json!({
                                "cve_id": id,
                                "description_first_line": desc,
                                "cvss_v31_score": score,
                                "cvss_v31_severity": severity,
                                "published": published,
                            }))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let body = json!({
                "total_results": total,
                "returned": rows.len(),
                "limit": limit,
                "results": rows,
            })
            .to_string();
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Recent high-severity nginx CVEs",
                args: r#"{"keyword": "nginx", "published_after": "2024-01-01", "cvss_v3_min": 7.0, "limit": 5}"#,
                note: Some("Filter by date + severity for triage."),
            },
            SkillExample {
                title: "By keyword only",
                args: r#"{"keyword": "log4j", "limit": 3}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Triage CVEs for a specific product before deciding on patch priority.",
            "Find recent high-severity vulnerabilities in a dependency.",
            "Build a watchlist of CVEs above a CVSS threshold.",
        ]
    }
}

fn severity_for(min: f64) -> String {
    if min >= 9.0 {
        "CRITICAL".into()
    } else if min >= 7.0 {
        "HIGH".into()
    } else if min >= 4.0 {
        "MEDIUM".into()
    } else {
        "LOW".into()
    }
}

// ---------------------------------------------------------------------------
// cpe_search
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CpeSearchArgs {
    /// Keyword to search CPE titles (e.g. `nginx`, `wordpress`).
    keyword: String,
    /// Max rows returned. Defaults to 20; capped at 50.
    #[serde(default)]
    limit: Option<u32>,
}

pub struct CpeSearch;
impl Skill for CpeSearch {
    fn name(&self) -> &'static str {
        "cpe_search"
    }
    fn description(&self) -> &'static str {
        "Find the canonical CPE 2.3 id for a product (vendor:product:version) via NVD's CPE \
         dictionary. Pass the resulting `cpeName` into `cve_search { cpe: ... }` to filter \
         vulnerabilities to that specific product version. Keyless."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CpeSearchArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<CpeSearchArgs>()?;
            let limit = args.limit.unwrap_or(20).min(50);
            let key = format!("cpe_search|{}|{limit}", args.keyword);
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let v: Value = server
                .http
                .get(CPE_API)
                .query(&[
                    ("keywordSearch", args.keyword.as_str()),
                    ("resultsPerPage", &limit.to_string()),
                ])
                .send()
                .await
                .map_err(|e| internal(e.into()))?
                .error_for_status()
                .map_err(|e| internal(e.into()))?
                .json()
                .await
                .map_err(|e| internal(e.into()))?;
            let products: Vec<Value> = v["products"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|p| {
                            let cpe = p.get("cpe")?;
                            let name = cpe["cpeName"].as_str()?.to_string();
                            let title = cpe["titles"]
                                .as_array()?
                                .iter()
                                .find(|t| t["lang"] == "en")
                                .and_then(|t| t["title"].as_str())
                                .map(str::to_string);
                            Some(json!({
                                "cpe_name": name,
                                "title": title,
                            }))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let body = json!({
                "keyword": args.keyword,
                "total_results": v.get("totalResults").and_then(|x| x.as_u64()),
                "returned": products.len(),
                "products": products,
            })
            .to_string();
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Find nginx",
                args: r#"{"keyword": "nginx", "limit": 5}"#,
                note: Some(
                    "Returns CPE 2.3 ids per nginx version — use one of these in cve_search.",
                ),
            },
            SkillExample {
                title: "WordPress",
                args: r#"{"keyword": "wordpress", "limit": 3}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Discover the canonical CPE id for a product before calling cve_search.",
            "Confirm which version strings NVD recognizes for a product.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "cve"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "NIST NVD lookups: one CVE record by id, search by keyword + CPE + date + severity, and \
         CPE 2.3 id discovery. Keyless. Backed by the public NVD v2.0 API; rate-limited (~5 req \
         / 30 sec without a key) so results are cached per the per-source TTL."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `cpe_search { keyword: \"nginx\" }` — find the canonical CPE id for nginx.\n\
             2. `cve_search { cpe: \"<chosen cpe>\", cvss_v3_min: 7.0 }` — high-severity issues for that version.\n\
             3. `cve_get { cve_id: \"<one hit>\" }` — drill into one record for the CVSS vector + references.",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(CveGet), Box::new(CveSearch), Box::new(CpeSearch)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_uppercases() {
        assert_eq!(normalize_cve("cve-2021-44228").unwrap(), "CVE-2021-44228");
    }

    #[test]
    fn rejects_non_cve() {
        assert!(normalize_cve("bug-123").is_err());
    }

    #[test]
    fn severity_floor() {
        assert_eq!(severity_for(9.5), "CRITICAL");
        assert_eq!(severity_for(7.0), "HIGH");
        assert_eq!(severity_for(5.0), "MEDIUM");
        assert_eq!(severity_for(2.0), "LOW");
    }
}
