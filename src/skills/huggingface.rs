//! Hugging Face Hub skills (keyless): `hf_search` searches models or datasets,
//! `hf_model` fetches one model's metadata. Uses the public `huggingface.co/api`
//! JSON endpoints — no token (a token would only be needed for private/gated repos).

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::human_count;
use crate::{clamp, internal, text_result};

async fn api_get(http: &Client, url: &str) -> Result<Value> {
    Ok(http
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Pull `license:x` / `arxiv:x` style values out of a HF tag list.
fn tag_value<'a>(tags: &'a [Value], prefix: &str) -> Option<&'a str> {
    tags.iter()
        .filter_map(|t| t.as_str())
        .find_map(|t| t.strip_prefix(prefix))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HfSearchArgs {
    /// What to search for (model/dataset name or keyword).
    query: String,
    /// What to search: "model" (default) or "dataset".
    #[serde(default)]
    kind: Option<String>,
    /// Maximum number of results. Default 10, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HfModelArgs {
    /// A model id, e.g. `google-bert/bert-base-uncased` or `gpt2`.
    model: String,
}

pub struct HfSearch;
impl Skill for HfSearch {
    fn name(&self) -> &'static str {
        "hf_search"
    }
    fn description(&self) -> &'static str {
        "Search the Hugging Face Hub (keyless) for models or datasets. `kind` is \"model\" \
        (default) or \"dataset\". Returns id, downloads, likes, and task/pipeline, sorted by \
        downloads. Use hf_model for one model's details."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HfSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<HfSearchArgs>()?;
            let limit = clamp(args.max_results, 10, 25);
            let dataset = matches!(
                args.kind
                    .as_deref()
                    .map(str::trim)
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("dataset") | Some("datasets")
            );
            let endpoint = if dataset { "datasets" } else { "models" };
            let url = format!(
                "https://huggingface.co/api/{endpoint}?search={}&limit={limit}&sort=downloads&direction=-1",
                urlencoding(&args.query)
            );
            let v = api_get(&server.http, &url).await.map_err(internal)?;
            let empty = Vec::new();
            let items = v.as_array().unwrap_or(&empty);
            if items.is_empty() {
                return Ok(text_result(format!("No {endpoint} match: {}", args.query)));
            }
            let mut out = format!("Hugging Face {endpoint} for \"{}\":\n", args.query);
            for it in items.iter().take(limit) {
                let id = it.get("id").and_then(|x| x.as_str()).unwrap_or("");
                let downloads = it.get("downloads").and_then(|x| x.as_i64()).unwrap_or(0);
                let likes = it.get("likes").and_then(|x| x.as_i64()).unwrap_or(0);
                let link = if dataset {
                    format!("https://huggingface.co/datasets/{id}")
                } else {
                    format!("https://huggingface.co/{id}")
                };
                out.push_str(&format!(
                    "\n{id}\n   {link}\n   ↓ {} · ♥ {}",
                    human_count(downloads),
                    human_count(likes)
                ));
                if let Some(task) = it.get("pipeline_tag").and_then(|x| x.as_str()) {
                    out.push_str(&format!(" · {task}"));
                }
                out.push('\n');
            }
            Ok(text_result(out))
        })
    }
}

pub struct HfModel;
impl Skill for HfModel {
    fn name(&self) -> &'static str {
        "hf_model"
    }
    fn description(&self) -> &'static str {
        "Get a Hugging Face model's metadata (keyless): downloads, likes, task/pipeline, library, \
        license, and tags. Accepts a model id like `google-bert/bert-base-uncased`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HfModelArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<HfModelArgs>()?;
            let id = args.model.trim().trim_start_matches('/');
            let url = format!("https://huggingface.co/api/models/{id}");
            let v = api_get(&server.http, &url).await.map_err(internal)?;
            let model_id = v.get("id").and_then(|x| x.as_str()).unwrap_or(id);
            let empty = Vec::new();
            let tags: Vec<Value> = v
                .get("tags")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or(empty);
            let mut out = format!("Hugging Face model: {model_id}\n");
            out.push_str(&format!("  https://huggingface.co/{model_id}\n"));
            if let Some(d) = v.get("downloads").and_then(|x| x.as_i64()) {
                out.push_str(&format!("  downloads: {}\n", human_count(d)));
            }
            if let Some(l) = v.get("likes").and_then(|x| x.as_i64()) {
                out.push_str(&format!("  likes: {}\n", human_count(l)));
            }
            if let Some(t) = v.get("pipeline_tag").and_then(|x| x.as_str()) {
                out.push_str(&format!("  task: {t}\n"));
            }
            if let Some(lib) = v.get("library_name").and_then(|x| x.as_str()) {
                out.push_str(&format!("  library: {lib}\n"));
            }
            if let Some(lic) = tag_value(&tags, "license:") {
                out.push_str(&format!("  license: {lic}\n"));
            }
            if let Some(m) = v
                .get("lastModified")
                .and_then(|x| x.as_str())
                .and_then(|d| d.get(..10))
            {
                out.push_str(&format!("  last modified: {m}\n"));
            }
            let topic_tags: Vec<&str> = tags
                .iter()
                .filter_map(|t| t.as_str())
                .filter(|t| !t.contains(':'))
                .take(12)
                .collect();
            if !topic_tags.is_empty() {
                out.push_str(&format!("  tags: {}\n", topic_tags.join(", ")));
            }
            Ok(text_result(out))
        })
    }
}

/// Minimal percent-encoding for a query string value (alnum + a few safe chars
/// pass through; everything else is %XX). Avoids a url-crate dep here.
fn urlencoding(s: &str) -> String {
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

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(HfSearch), Box::new(HfModel)]
}
