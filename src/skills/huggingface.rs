//! Hugging Face Hub skills (keyless): `hf_model_search` and `hf_dataset_search`
//! each search one corpus (no hidden mode flag); `hf_model` fetches one model's
//! metadata. Uses the public `huggingface.co/api` JSON endpoints — no token (a
//! token would only be needed for private/gated repos).

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
    /// What to search for (name or keyword).
    query: String,
    /// Maximum number of results. Default 10, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HfModelArgs {
    /// A model id, e.g. `google-bert/bert-base-uncased` or `gpt2`.
    model: String,
}

/// Search one Hub corpus (`models` or `datasets`), sorted by downloads.
async fn hf_search(
    server: &crate::Lodestone,
    query: &str,
    max_results: Option<u32>,
    dataset: bool,
) -> Result<String, McpError> {
    let limit = clamp(max_results, 10, 25);
    let endpoint = if dataset { "datasets" } else { "models" };
    let cache_key = format!("hf_search|{endpoint}|{limit}|{}", query.trim());
    if let Some(cached) = server.retrieval_get(&cache_key).await {
        return Ok(cached);
    }
    let url = format!(
        "https://huggingface.co/api/{endpoint}?search={}&limit={limit}&sort=downloads&direction=-1",
        urlencoding(query)
    );
    let v = api_get(&server.http, &url).await.map_err(internal)?;
    let empty = Vec::new();
    let items = v.as_array().unwrap_or(&empty);
    if items.is_empty() {
        return Ok(format!("No {endpoint} match: {query}"));
    }
    let mut out = format!("Hugging Face {endpoint} for \"{query}\":\n");
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
    server.retrieval_put(cache_key, &out);
    Ok(out)
}

pub struct HfModelSearch;
impl Skill for HfModelSearch {
    fn name(&self) -> &'static str {
        "hf_model_search"
    }
    fn description(&self) -> &'static str {
        "Search the Hugging Face Hub for MODELS (keyless). Returns id, downloads, likes, and \
        task/pipeline, sorted by downloads. Use hf_dataset_search for datasets, or hf_model for \
        one model's full details."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HfSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<HfSearchArgs>()?;
            Ok(text_result(
                hf_search(server, &args.query, args.max_results, false).await?,
            ))
        })
    }
}

pub struct HfDatasetSearch;
impl Skill for HfDatasetSearch {
    fn name(&self) -> &'static str {
        "hf_dataset_search"
    }
    fn description(&self) -> &'static str {
        "Search the Hugging Face Hub for DATASETS (keyless). Returns id, downloads, likes, sorted \
        by downloads. Use hf_model_search for models."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HfSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<HfSearchArgs>()?;
            Ok(text_result(
                hf_search(server, &args.query, args.max_results, true).await?,
            ))
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
            let cache_key = format!("hf_model|{id}");
            if let Some(cached) = server.retrieval_get(&cache_key).await {
                return Ok(text_result(cached));
            }
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
            server.retrieval_put(cache_key, &out);
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
    vec![
        Box::new(HfModelSearch),
        Box::new(HfDatasetSearch),
        Box::new(HfModel),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoding_safe_chars_passthrough() {
        assert_eq!(urlencoding("bert-base"), "bert-base");
        assert_eq!(urlencoding("a/b"), "a%2Fb");
        assert_eq!(urlencoding("hello world"), "hello%20world");
    }

    #[test]
    fn tag_value_extracts_prefix() {
        let tags = vec![
            serde_json::Value::String("license:apache-2.0".into()),
            serde_json::Value::String("arxiv:2305.15334".into()),
            serde_json::Value::String("text-generation".into()),
        ];
        assert_eq!(tag_value(&tags, "license:"), Some("apache-2.0"));
        assert_eq!(tag_value(&tags, "arxiv:"), Some("2305.15334"));
        assert_eq!(tag_value(&tags, "missing:"), None);
    }

    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent("lodestone-mcp/0.1.0 (+https://github.com/elyerinfox/lodestone-mcp)")
            .build()
            .unwrap()
    }

    /// `gpt2` is canonical and won't go away — stable target for the live test.
    #[tokio::test]
    #[ignore]
    async fn hf_model_live() {
        let r = http()
            .get("https://huggingface.co/api/models/gpt2")
            .header("Accept", "application/json")
            .send().await.expect("network").error_for_status().unwrap();
        let v: Value = r.json().await.unwrap();
        let id = v["id"].as_str().expect("id field missing");
        // HF reorganized many models under org namespaces; gpt2 now resolves to
        // openai-community/gpt2. Accept either form so the test stays useful
        // when they migrate again.
        assert!(id.ends_with("gpt2"), "got id={id:?}");
        // Fields the skill renders:
        for k in ["downloads", "likes", "tags"] {
            assert!(v.get(k).is_some(), "missing field {k}");
        }
    }

    #[tokio::test]
    #[ignore]
    async fn hf_model_search_live() {
        let r = http()
            .get("https://huggingface.co/api/models?search=bert&limit=3&sort=downloads&direction=-1")
            .header("Accept", "application/json")
            .send().await.expect("network").error_for_status().unwrap();
        let v: Value = r.json().await.unwrap();
        let arr = v.as_array().expect("expected JSON array");
        assert!(!arr.is_empty());
        assert!(arr[0].get("id").is_some());
    }

    #[tokio::test]
    #[ignore]
    async fn hf_dataset_search_live() {
        let r = http()
            .get("https://huggingface.co/api/datasets?search=squad&limit=3&sort=downloads&direction=-1")
            .header("Accept", "application/json")
            .send().await.expect("network").error_for_status().unwrap();
        let v: Value = r.json().await.unwrap();
        let arr = v.as_array().expect("expected JSON array");
        assert!(!arr.is_empty());
        assert!(arr[0].get("id").is_some());
    }
}
