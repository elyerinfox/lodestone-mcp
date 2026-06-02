//! `artifacthub_search` skill — Artifact Hub (keyless), the CNCF index of
//! Kubernetes-ecosystem packages: Helm charts, Operators (OLM), Falco rules,
//! OPA/Kyverno/Gatekeeper policies, krew kubectl plugins, Tekton tasks, and more.
//!
//! Golden rule: keyless and plain-HTTP. `GET /api/v1/packages/search` returns
//! JSON; no account or key.

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

const SEARCH: &str = "https://artifacthub.io/api/v1/packages/search";

/// Search Artifact Hub. `kind` optionally restricts to one package kind slug
/// (e.g. "helm", "olm", "krew"). Returns the raw JSON (`packages` array).
pub async fn search(http: &Client, query: &str, kind: Option<&str>, limit: usize) -> Result<Value> {
    let size = limit.clamp(1, 60).to_string();
    let mut params: Vec<(&str, String)> = vec![
        ("ts_query_web", query.to_string()),
        ("limit", size),
        ("offset", "0".to_string()),
        ("facets", "false".to_string()),
    ];
    if let Some(k) = kind.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(n) = kind_num(k) {
            params.push(("kind", n.to_string()));
        }
    }
    Ok(http
        .get(SEARCH)
        .query(&params)
        .header("Accept", "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Build the artifacthub.io page URL for a search-result package.
pub fn package_url(pkg: &Value) -> String {
    let name = pkg
        .get("normalized_name")
        .or_else(|| pkg.get("name"))
        .and_then(|x| x.as_str());
    let repo = pkg
        .get("repository")
        .and_then(|r| r.get("name"))
        .and_then(|x| x.as_str());
    let kind = pkg
        .get("repository")
        .and_then(|r| r.get("kind"))
        .and_then(|x| x.as_u64());
    match (kind.and_then(kind_slug), repo, name) {
        (Some(slug), Some(repo), Some(name)) => {
            format!("https://artifacthub.io/packages/{slug}/{repo}/{name}")
        }
        (_, _, Some(name)) => {
            format!("https://artifacthub.io/packages/search?ts_query_web={name}")
        }
        _ => "https://artifacthub.io/".to_string(),
    }
}

/// Map an Artifact Hub repository-kind number to its URL slug.
pub fn kind_slug(n: u64) -> Option<&'static str> {
    Some(match n {
        0 => "helm",
        1 => "falco",
        2 => "opa",
        3 => "olm",
        4 => "tbaction",
        5 => "krew",
        6 => "helm-plugin",
        7 => "tekton-task",
        8 => "keda-scaler",
        9 => "coredns",
        10 => "keptn",
        11 => "tekton-pipeline",
        12 => "container",
        13 => "kubewarden",
        14 => "gatekeeper",
        15 => "kyverno",
        16 => "knative-client-plugin",
        17 => "backstage",
        18 => "argo-template",
        19 => "kubearmor",
        20 => "kcl",
        21 => "headlamp",
        22 => "inspektor-gadget",
        23 => "tekton-stepaction",
        24 => "meshery",
        25 => "opencost",
        26 => "radius",
        27 => "bootc",
        _ => return None,
    })
}

/// Map a kind slug back to its number, for the search `kind` filter.
fn kind_num(slug: &str) -> Option<u64> {
    (0..=27).find(|&n| kind_slug(n) == Some(slug))
}

fn format_results(query: &str, kind: Option<&str>, v: &Value, limit: usize) -> String {
    let scope = kind.map(|k| format!(" [{k}]")).unwrap_or_default();
    let mut out = format!("Artifact Hub results for \"{query}\"{scope}:\n");
    let empty = Vec::new();
    let packages = v
        .get("packages")
        .and_then(|x| x.as_array())
        .unwrap_or(&empty);
    if packages.is_empty() {
        out.push_str("\n(no packages found)");
        return out;
    }
    for (i, p) in packages.iter().take(limit).enumerate() {
        let name = p.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let version = p.get("version").and_then(|x| x.as_str()).unwrap_or("");
        let stars = p.get("stars").and_then(|x| x.as_i64()).unwrap_or(0);
        let desc = p.get("description").and_then(|x| x.as_str()).unwrap_or("");
        let repo = p.get("repository");
        let kind_slug = repo
            .and_then(|r| r.get("kind"))
            .and_then(|x| x.as_u64())
            .and_then(kind_slug)
            .unwrap_or("package");
        let publisher = repo
            .and_then(|r| {
                r.get("organization_name")
                    .or_else(|| r.get("user_alias"))
                    .or_else(|| r.get("name"))
            })
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let url = package_url(p);
        out.push_str(&format!("\n{}. {name}", i + 1));
        if !version.is_empty() {
            out.push_str(&format!(" {version}"));
        }
        out.push_str(&format!(" [{kind_slug}]\n"));
        out.push_str(&format!("   {url}\n"));
        let mut facts = Vec::new();
        if !publisher.is_empty() {
            facts.push(format!("by {publisher}"));
        }
        if stars > 0 {
            facts.push(format!("★ {}", human_count(stars)));
        }
        if !facts.is_empty() {
            out.push_str(&format!("   {}\n", facts.join(" · ")));
        }
        if !desc.is_empty() {
            out.push_str(&format!("   {desc}\n"));
        }
    }
    out
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ArtifactHubArgs {
    /// What to search for (chart/operator/plugin name or keyword).
    query: String,
    /// Optional package-kind filter: helm, olm, krew, falco, opa, kyverno,
    /// gatekeeper, tekton-task, coredns, container, … Omit to search all kinds.
    #[serde(default)]
    kind: Option<String>,
    /// Maximum number of results to return. Default 10, capped at 30.
    #[serde(default)]
    max_results: Option<u32>,
}

pub struct ArtifactHubSearch;

impl Skill for ArtifactHubSearch {
    fn name(&self) -> &'static str {
        "artifacthub_search"
    }
    fn description(&self) -> &'static str {
        "Search Artifact Hub (keyless) — the index of Kubernetes-ecosystem packages: Helm charts, \
        Operators, krew plugins, Falco/OPA/Kyverno/Gatekeeper policies, Tekton tasks, and more. \
        Optional `kind` filter (e.g. helm, olm, krew). Returns name, version, stars, publisher, link."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ArtifactHubArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }

    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ArtifactHubArgs>()?;
            let limit = clamp(args.max_results, 10, 30);
            let kind = args
                .kind
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let key = format!("artifacthub|{limit}|{}|{}", kind.unwrap_or(""), args.query);
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let v = search(&server.http, &args.query, kind, limit)
                .await
                .map_err(internal)?;
            let out = format_results(&args.query, kind, &v, limit);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(ArtifactHubSearch)]
}

#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        crate::skills::live_http()
    }

    #[tokio::test]
    #[ignore]
    async fn artifacthub_search_live() {
        let r = http()
            .get("https://artifacthub.io/api/v1/packages/search?ts_query_web=nginx&limit=3")
            .header("Accept", "application/json")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        let pkgs = v["packages"].as_array().expect("missing packages");
        assert!(!pkgs.is_empty());
        for k in ["package_id", "name", "repository"] {
            assert!(pkgs[0].get(k).is_some(), "missing field {k}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_round_trips() {
        assert_eq!(kind_slug(0), Some("helm"));
        assert_eq!(kind_num("helm"), Some(0));
        assert_eq!(kind_num("krew"), Some(5));
        assert_eq!(kind_num("nope"), None);
        assert_eq!(kind_slug(999), None);
    }

    #[test]
    fn builds_package_url_from_kind() {
        let pkg = serde_json::json!({
            "name": "ingress-nginx",
            "normalized_name": "ingress-nginx",
            "repository": {"name": "ingress-nginx", "kind": 0}
        });
        assert_eq!(
            package_url(&pkg),
            "https://artifacthub.io/packages/helm/ingress-nginx/ingress-nginx"
        );
    }

    #[test]
    fn unknown_kind_falls_back_to_search() {
        let pkg = serde_json::json!({"name": "x", "repository": {"name": "r", "kind": 999}});
        assert!(package_url(&pkg).contains("/packages/search?"));
    }
}
