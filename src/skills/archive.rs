//! `wayback_fetch` skill — the Internet Archive Wayback Machine (keyless). Reads
//! the closest archived snapshot of a URL, useful when a page is down, paywalled,
//! changed, or blocking automated access, or to view a historical version.
//!
//! Owns the Wayback client; it reuses [`crate::skills::retrieve::fetch_readable`]
//! to turn the resolved snapshot into readable text.

use std::sync::{Arc, LazyLock};

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use regex::Regex;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::retrieve::fetch_readable;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, text_result};

static WAYBACK_TS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/web/(\d+)(?:[a-z_]+)?/").unwrap());

/// Look up the closest archived snapshot for `url` (optionally near `timestamp`,
/// a `YYYYMMDD[hhmmss]` string). Returns a direct, toolbar-free snapshot URL.
async fn wayback_snapshot(
    client: &Client,
    url: &str,
    timestamp: Option<&str>,
) -> Result<Option<String>> {
    let mut params = vec![("url", url)];
    if let Some(ts) = timestamp {
        params.push(("timestamp", ts));
    }
    let v: serde_json::Value = client
        .get("https://archive.org/wayback/available")
        .query(&params)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let available = v
        .pointer("/archived_snapshots/closest/available")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    if !available {
        return Ok(None);
    }
    let snapshot = v
        .pointer("/archived_snapshots/closest/url")
        .and_then(|x| x.as_str())
        .map(to_raw_snapshot);
    Ok(snapshot)
}

/// Resolve a snapshot for `url` and return `(snapshot_url, readable_text)`.
async fn wayback_fetch(
    client: &Client,
    url: &str,
    timestamp: Option<&str>,
    max_chars: usize,
) -> Result<(String, String)> {
    let snapshot = wayback_snapshot(client, url, timestamp)
        .await?
        .ok_or_else(|| anyhow!("no archived snapshot found for {url}"))?;
    let text = fetch_readable(client, &snapshot, max_chars).await?;
    Ok((snapshot, text))
}

/// Turn a Wayback viewer URL into the raw-content form (insert `id_` after the
/// timestamp so the archive toolbar/rewriting is omitted) over HTTPS.
fn to_raw_snapshot(url: &str) -> String {
    let url = WAYBACK_TS_RE.replace(url, "/web/${1}id_/");
    if let Some(rest) = url.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        url.into_owned()
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WaybackFetchArgs {
    /// Absolute URL to look up in the Internet Archive Wayback Machine.
    url: String,
    /// Optional snapshot target as `YYYYMMDD` or `YYYYMMDDhhmmss`; the closest
    /// capture is returned. Omit for the most recent snapshot.
    #[serde(default)]
    timestamp: Option<String>,
    /// Max characters of extracted text to return. Omit for the server default;
    /// capped by the server's `[retrieval].max_chars`. Increase for full pages.
    #[serde(default)]
    max_chars: Option<u32>,
}

pub struct WaybackFetch;
impl Skill for WaybackFetch {
    fn name(&self) -> &'static str {
        "wayback_fetch"
    }
    fn description(&self) -> &'static str {
        "Fetch a page from the Internet Archive Wayback Machine (keyless). Returns the readable \
        text of the closest archived snapshot. Useful when a page is down, paywalled, changed, or \
        blocking automated access, or to view a historical version. Output is truncated to a \
        character budget — pass a larger `max_chars` to get more."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WaybackFetchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<WaybackFetchArgs>()?;
            let max = server.clamp_chars(args.max_chars);
            let key = format!(
                "wayback|{max}|{}|{}",
                args.timestamp.as_deref().unwrap_or(""),
                args.url
            );
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let (snapshot, text) =
                wayback_fetch(&server.http, &args.url, args.timestamp.as_deref(), max)
                    .await
                    .map_err(internal)?;
            let out = format!("Source (archived): {snapshot}\n\n{text}");
            if !text.is_empty() {
                server.retrieval_put(key, &out);
            }
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(WaybackFetch)]
}
