//! Artifact Hub search (keyless) — the CNCF index of Kubernetes-ecosystem
//! packages: Helm charts, Operators (OLM), Falco rules, OPA/Kyverno/Gatekeeper
//! policies, krew kubectl plugins, Tekton tasks, and more.
//!
//! Golden rule: keyless and plain-HTTP. `GET /api/v1/packages/search` returns
//! JSON; no account or key. Exposed as the `artifacthub_search` skill.

use anyhow::Result;
use reqwest::Client;
use serde_json::Value;

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
        // Unknown kind — fall back to a search link that lands on the package.
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
