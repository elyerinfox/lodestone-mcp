//! IETF RFC skills (keyless): `rfc_get` fetches an RFC's full text directly from
//! the RFC Editor (`rfc-editor.org/rfc/rfcN.txt`), and `rfc_search` finds RFCs by
//! title via the IETF Datatracker's JSON API. No account or key.

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
use crate::{clamp, internal, invalid, text_result};

/// Parse an RFC number from `9110`, `rfc9110`, `RFC 9110`, etc.
fn rfc_number(input: &str) -> Option<u32> {
    let s = input.trim().to_ascii_lowercase();
    let s = s.strip_prefix("rfc").unwrap_or(&s).trim();
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok().filter(|&n| n > 0)
}

/// Fetch an RFC's full text from the RFC Editor. `Ok(None)` when it doesn't exist.
async fn fetch_rfc_text(client: &Client, n: u32) -> Result<Option<String>> {
    let url = format!("https://www.rfc-editor.org/rfc/rfc{n}.txt");
    let resp = client.get(&url).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    Ok(Some(resp.error_for_status()?.text().await?))
}

/// Search RFCs by title via the IETF Datatracker document API.
async fn search_rfcs(client: &Client, query: &str, limit: usize) -> Result<Value> {
    let lim = limit.to_string();
    Ok(client
        .get("https://datatracker.ietf.org/api/v1/doc/document/")
        .query(&[
            ("format", "json"),
            ("type", "rfc"),
            ("title__icontains", query),
            ("limit", lim.as_str()),
        ])
        .header("Accept", "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RfcGetArgs {
    /// The RFC to fetch: a number or `rfc`-prefixed id, e.g. `9110`, `rfc9110`,
    /// `RFC 9110`.
    document: String,
    /// Max characters of text to return. Omit for the server default; RFCs can be
    /// long, so increase this (or call again) to read further.
    #[serde(default)]
    max_chars: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct RfcSearchArgs {
    /// Words to match in the RFC title (e.g. "http semantics", "tls").
    query: String,
    /// Maximum number of results. Default 10, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
}

pub struct RfcGet;
impl Skill for RfcGet {
    fn name(&self) -> &'static str {
        "rfc_get"
    }
    fn description(&self) -> &'static str {
        "Fetch an IETF RFC's full text directly from the RFC Editor (keyless). Accepts a number or \
        `rfc`-prefixed id (e.g. 9110, rfc9110, 'RFC 9110'). Output is truncated to a character \
        budget — pass a larger `max_chars` to read more of a long RFC."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RfcGetArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<RfcGetArgs>()?;
            let n = rfc_number(&args.document)
                .ok_or_else(|| invalid(format!("not an RFC number: '{}'", args.document)))?;
            let max = server.clamp_chars(args.max_chars);
            let key = format!("rfc|{max}|{n}");
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let text = fetch_rfc_text(&server.http, n).await.map_err(internal)?;
            let Some(text) = text else {
                return Ok(text_result(format!("RFC {n} not found.")));
            };
            let url = format!("https://www.rfc-editor.org/rfc/rfc{n}.txt");
            let out = format!("RFC {n} — {url}\n\n{}", truncate_chars(&text, max));
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct RfcSearch;
impl Skill for RfcSearch {
    fn name(&self) -> &'static str {
        "rfc_search"
    }
    fn description(&self) -> &'static str {
        "Search IETF RFCs by title via the Datatracker (keyless). Returns matching RFCs with \
        number, title, and abstract. Then use rfc_get to read one's full text."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<RfcSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<RfcSearchArgs>()?;
            let limit = clamp(args.max_results, 10, 25);
            let v = search_rfcs(&server.http, &args.query, limit)
                .await
                .map_err(internal)?;
            let empty = Vec::new();
            let objects = v
                .get("objects")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            if objects.is_empty() {
                return Ok(text_result(format!("No RFCs match: {}", args.query)));
            }
            let mut out = format!("RFCs matching \"{}\":\n", args.query);
            for o in objects.iter().take(limit) {
                let name = o.get("name").and_then(|x| x.as_str()).unwrap_or("");
                let title = o.get("title").and_then(|x| x.as_str()).unwrap_or("");
                let label = name
                    .strip_prefix("rfc")
                    .map(|n| format!("RFC {n}"))
                    .unwrap_or_else(|| name.to_string());
                out.push_str(&format!(
                    "\n{label}: {title}\n   https://www.rfc-editor.org/rfc/{name}\n"
                ));
                if let Some(abs) = o
                    .get("abstract")
                    .and_then(|x| x.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    out.push_str(&format!("   {}\n", truncate_chars(abs, 300)));
                }
            }
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(RfcGet), Box::new(RfcSearch)]
}

#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent("lodestone-mcp/0.1.0 (+https://github.com/elyerinfox/lodestone-mcp)")
            .build()
            .unwrap()
    }

    /// RFC Editor txt format — stable URL. RFC 2616 = HTTP/1.1.
    #[tokio::test]
    #[ignore]
    async fn rfc_get_live() {
        let r = http()
            .get("https://www.rfc-editor.org/rfc/rfc2616.txt")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let body = r.text().await.unwrap();
        assert!(
            body.contains("Hypertext Transfer Protocol"),
            "got non-RFC body"
        );
    }

    /// IETF Datatracker search-by-title — JSON envelope.
    #[tokio::test]
    #[ignore]
    async fn ietf_datatracker_search_live() {
        let r = http()
            .get("https://datatracker.ietf.org/api/v1/doc/document/?type=rfc&title__contains=HTTP&limit=3&format=json")
            .send().await.expect("network").error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        let objects = v["objects"].as_array().expect("missing objects array");
        assert!(!objects.is_empty());
        assert!(objects[0].get("title").is_some());
    }
}

#[cfg(test)]
mod tests {
    use super::rfc_number;

    #[test]
    fn parses_rfc_numbers() {
        assert_eq!(rfc_number("9110"), Some(9110));
        assert_eq!(rfc_number("rfc9110"), Some(9110));
        assert_eq!(rfc_number("RFC 9110"), Some(9110));
        assert_eq!(rfc_number("rfc 791 (IP)"), Some(791));
        assert_eq!(rfc_number("not an rfc"), None);
        assert_eq!(rfc_number("rfc0"), None);
    }
}
