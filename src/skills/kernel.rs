//! kernel.org skill (keyless): `kernel_releases` lists the current Linux kernel
//! releases (mainline / stable / longterm, with dates and EOL status) from
//! kernel.org's published `releases.json`. Kernel documentation is searchable via
//! the `docs_kernel` doc-site provider; source tarballs are at the `source` links.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde_json::Value;

use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::{internal, text_result};

async fn releases(http: &Client) -> Result<Value> {
    Ok(http
        .get("https://www.kernel.org/releases.json")
        .header("Accept", "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

pub struct KernelReleases;
impl Skill for KernelReleases {
    fn name(&self) -> &'static str {
        "kernel_releases"
    }
    fn description(&self) -> &'static str {
        "List the current Linux kernel releases from kernel.org (keyless): mainline, stable, and \
        longterm versions with their release dates, EOL status, and source-tarball links. Use to \
        answer 'what's the latest/longterm kernel'."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let v = releases(&ctx.server.http).await.map_err(internal)?;
            let empty = Vec::new();
            let list = v
                .get("releases")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            if list.is_empty() {
                return Ok(text_result("No kernel releases reported."));
            }
            let mut out = String::from("Linux kernel releases (kernel.org):\n");
            for r in list {
                let moniker = r.get("moniker").and_then(|x| x.as_str()).unwrap_or("");
                let version = r.get("version").and_then(|x| x.as_str()).unwrap_or("");
                let date = r
                    .pointer("/released/isodate")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                let eol = r.get("iseol").and_then(|x| x.as_bool()).unwrap_or(false);
                let src = r.get("source").and_then(|x| x.as_str()).unwrap_or("");
                out.push_str(&format!(
                    "\n  {moniker:<10} {version:<14} {date}{}",
                    if eol { "  [EOL]" } else { "" }
                ));
                if !src.is_empty() {
                    out.push_str(&format!("\n             {src}"));
                }
            }
            if let Some(latest) = v.pointer("/latest_stable/version").and_then(|x| x.as_str()) {
                out.push_str(&format!("\n\nlatest stable: {latest}"));
            }
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(KernelReleases)]
}
