//! Kubernetes cluster interaction via the API server (kube-rs). Reads your
//! kubeconfig (default location, `$KUBECONFIG`, or a configured path/context) or
//! in-cluster service-account credentials — no `kubectl` binary.
//!
//! A local/cluster-control capability, separate from the keyless web tools. Gated
//! by `[kubernetes]` (on by default); destructive actions (`k8s_delete`) are hidden
//! unless `allow_destructive` is set. Every action is its own tool for per-action
//! permission granularity. kube types are fully encapsulated here (functions return
//! formatted `String`s), so `main.rs` never depends on kube.

use anyhow::{anyhow, Context as _, Result};
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams, ListParams, LogParams, Patch, PatchParams};
use kube::config::{KubeConfigOptions, Kubeconfig};
use kube::core::DynamicObject;
use kube::discovery::{ApiResource, Discovery, Scope};
use kube::{Client, Config};
use serde::Deserialize;
use serde_json::Value;

/// Connection options resolved from `[kubernetes]` config.
pub struct Opts {
    pub kubeconfig: String,
    pub context: String,
    pub namespace: String,
}

impl Opts {
    /// Resolve the namespace: a per-call override, else the configured default,
    /// else `"default"`.
    fn ns<'a>(&'a self, over: Option<&'a str>) -> &'a str {
        over.map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                let n = self.namespace.trim();
                if n.is_empty() {
                    "default"
                } else {
                    n
                }
            })
    }
}

/// Build a Kubernetes client from the configured kubeconfig/context, or infer it
/// (default kubeconfig / `$KUBECONFIG` / in-cluster) when nothing is configured.
async fn client(opts: &Opts) -> Result<Client> {
    let ctx = opts.context.trim();
    let path = opts.kubeconfig.trim();
    let config = if ctx.is_empty() && path.is_empty() {
        Config::infer()
            .await
            .context("loading Kubernetes config (kubeconfig or in-cluster)")?
    } else {
        let kc = if path.is_empty() {
            Kubeconfig::read()
        } else {
            Kubeconfig::read_from(path)
        }
        .context("reading kubeconfig")?;
        let options = KubeConfigOptions {
            context: (!ctx.is_empty()).then(|| ctx.to_string()),
            ..Default::default()
        };
        Config::from_custom_kubeconfig(kc, &options)
            .await
            .context("building Kubernetes config")?
    };
    Client::try_from(config).context("building Kubernetes client")
}

/// Resolve a user-supplied kind (e.g. "pods", "deploy", "Service") to its API
/// resource and scope via discovery.
async fn resolve(client: &Client, kind: &str) -> Result<(ApiResource, Scope)> {
    let want = canonical_kind(&kind.trim().to_ascii_lowercase());
    let discovery = Discovery::new(client.clone())
        .run()
        .await
        .context("Kubernetes API discovery")?;
    for group in discovery.groups() {
        for (ar, caps) in group.recommended_resources() {
            let k = ar.kind.to_ascii_lowercase();
            let p = ar.plural.to_ascii_lowercase();
            if k == want || p == want {
                return Ok((ar, caps.scope));
            }
        }
    }
    Err(anyhow!("unknown resource kind '{kind}'"))
}

/// Map common kubectl short names / plurals to a canonical lowercase kind so they
/// match `ApiResource.kind`.
fn canonical_kind(k: &str) -> String {
    let s = match k {
        "po" | "pods" => "pod",
        "deploy" | "deployments" | "deployment" => "deployment",
        "svc" | "services" | "service" => "service",
        "ns" | "namespaces" | "namespace" => "namespace",
        "no" | "nodes" | "node" => "node",
        "cm" | "configmaps" | "configmap" => "configmap",
        "secrets" | "secret" => "secret",
        "rs" | "replicasets" | "replicaset" => "replicaset",
        "sts" | "statefulsets" | "statefulset" => "statefulset",
        "ds" | "daemonsets" | "daemonset" => "daemonset",
        "ing" | "ingresses" | "ingress" => "ingress",
        "pv" | "persistentvolumes" => "persistentvolume",
        "pvc" | "persistentvolumeclaims" => "persistentvolumeclaim",
        "jobs" | "job" => "job",
        "cj" | "cronjobs" | "cronjob" => "cronjob",
        "ep" | "endpoints" => "endpoints",
        "sa" | "serviceaccounts" | "serviceaccount" => "serviceaccount",
        other => other,
    };
    s.to_string()
}

fn dyn_api(client: &Client, ar: &ApiResource, scope: &Scope, ns: &str) -> Api<DynamicObject> {
    match scope {
        Scope::Namespaced => Api::namespaced_with(client.clone(), ns, ar),
        Scope::Cluster => Api::all_with(client.clone(), ar),
    }
}

/// List kubeconfig contexts and the current one (no cluster contact).
pub fn contexts(opts: &Opts) -> Result<String> {
    let path = opts.kubeconfig.trim();
    let kc = if path.is_empty() {
        Kubeconfig::read()
    } else {
        Kubeconfig::read_from(path)
    }
    .context("reading kubeconfig")?;
    let current = kc.current_context.unwrap_or_default();
    let mut out = String::from("Kubeconfig contexts:\n");
    for c in &kc.contexts {
        let marker = if c.name == current {
            "  * (current)"
        } else {
            ""
        };
        out.push_str(&format!("  {}{marker}\n", c.name));
    }
    if kc.contexts.is_empty() {
        out.push_str("  (none)\n");
    }
    Ok(out)
}

/// Get a resource: a single named object (full JSON), or a list (one line each).
pub async fn get(
    opts: &Opts,
    kind: &str,
    name: Option<&str>,
    namespace: Option<&str>,
) -> Result<String> {
    let client = client(opts).await?;
    let (ar, scope) = resolve(&client, kind).await?;
    let ns = opts.ns(namespace);
    let api = dyn_api(&client, &ar, &scope, ns);
    match name.map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => {
            let obj = api
                .get(n)
                .await
                .with_context(|| format!("getting {}/{n}", ar.kind))?;
            Ok(format!(
                "{}/{n}:\n{}",
                ar.kind,
                serde_json::to_string_pretty(&obj).unwrap_or_default()
            ))
        }
        None => {
            let list = api
                .list(&ListParams::default())
                .await
                .with_context(|| format!("listing {}", ar.plural))?;
            let scope_note = if matches!(scope, Scope::Namespaced) {
                format!(" in ns {ns}")
            } else {
                String::new()
            };
            let mut out = format!("{} ({} items{scope_note}):\n", ar.kind, list.items.len());
            for item in &list.items {
                let nm = item.metadata.name.as_deref().unwrap_or("?");
                let phase = item
                    .data
                    .get("status")
                    .and_then(|s| s.get("phase"))
                    .and_then(|p| p.as_str())
                    .map(|p| format!("  [{p}]"))
                    .unwrap_or_default();
                out.push_str(&format!("  {nm}{phase}\n"));
            }
            Ok(out)
        }
    }
}

/// Full JSON of one named resource (like `kubectl describe`, but the object).
pub async fn describe(
    opts: &Opts,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<String> {
    get(opts, kind, Some(name), namespace).await
}

/// A pod's logs (last `tail` lines; optional container).
pub async fn logs(
    opts: &Opts,
    pod: &str,
    namespace: Option<&str>,
    container: Option<&str>,
    tail: usize,
) -> Result<String> {
    let client = client(opts).await?;
    let ns = opts.ns(namespace);
    let api: Api<Pod> = Api::namespaced(client, ns);
    let lp = LogParams {
        tail_lines: Some(tail as i64),
        container: container.map(str::to_string),
        ..Default::default()
    };
    let logs = api
        .logs(pod, &lp)
        .await
        .with_context(|| format!("reading logs for pod '{pod}' (ns {ns})"))?;
    let body = if logs.trim().is_empty() {
        "(no logs)".to_string()
    } else {
        logs
    };
    Ok(format!("Logs for pod {pod} (ns {ns}):\n{body}"))
}

/// Server-side apply one or more manifest documents ("kubefiles", YAML).
pub async fn apply(opts: &Opts, manifest: &str) -> Result<String> {
    // Parse all documents up front: the YAML deserializer isn't `Send`, so it
    // must not be held across an `.await` below.
    let mut docs: Vec<Value> = Vec::new();
    for de in serde_yaml::Deserializer::from_str(manifest) {
        let yv = serde_yaml::Value::deserialize(de).context("parsing manifest YAML")?;
        let value: Value = serde_json::to_value(yv).context("converting manifest to JSON")?;
        if !value.is_null() {
            docs.push(value);
        }
    }

    let client = client(opts).await?;
    let pp = PatchParams::apply("lodestone-mcp").force();
    let mut results = Vec::new();
    for value in docs {
        let kind = value
            .get("kind")
            .and_then(|x| x.as_str())
            .ok_or_else(|| anyhow!("a manifest document is missing `kind`"))?
            .to_string();
        let name = value
            .get("metadata")
            .and_then(|m| m.get("name"))
            .and_then(|x| x.as_str())
            .ok_or_else(|| anyhow!("a {kind} document is missing metadata.name"))?
            .to_string();
        let ns_doc = value
            .get("metadata")
            .and_then(|m| m.get("namespace"))
            .and_then(|x| x.as_str());
        let (ar, scope) = resolve(&client, &kind).await?;
        let ns = opts.ns(ns_doc);
        let api = dyn_api(&client, &ar, &scope, ns);
        api.patch(&name, &pp, &Patch::Apply(value))
            .await
            .with_context(|| format!("applying {kind}/{name}"))?;
        results.push(format!("  {kind}/{name} applied"));
    }
    if results.is_empty() {
        return Err(anyhow!("no manifest documents found"));
    }
    Ok(format!(
        "Applied {} object(s):\n{}",
        results.len(),
        results.join("\n")
    ))
}

/// Scale a workload to `replicas` (deployment/statefulset/replicaset).
pub async fn scale(
    opts: &Opts,
    kind: &str,
    name: &str,
    replicas: i32,
    namespace: Option<&str>,
) -> Result<String> {
    let client = client(opts).await?;
    let (ar, scope) = resolve(&client, kind).await?;
    let ns = opts.ns(namespace);
    let api = dyn_api(&client, &ar, &scope, ns);
    let patch = serde_json::json!({ "spec": { "replicas": replicas } });
    api.patch(name, &PatchParams::default(), &Patch::Merge(patch))
        .await
        .with_context(|| format!("scaling {kind}/{name}"))?;
    Ok(format!("Scaled {kind}/{name} to {replicas} replicas"))
}

// --- destructive (gated by [kubernetes].allow_destructive) -------------------

/// Delete a named resource.
pub async fn delete(
    opts: &Opts,
    kind: &str,
    name: &str,
    namespace: Option<&str>,
) -> Result<String> {
    let client = client(opts).await?;
    let (ar, scope) = resolve(&client, kind).await?;
    let ns = opts.ns(namespace);
    let api = dyn_api(&client, &ar, &scope, ns);
    api.delete(name, &DeleteParams::default())
        .await
        .with_context(|| format!("deleting {kind}/{name}"))?;
    Ok(format!("Deleted {kind}/{name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_kind_maps_shortnames() {
        assert_eq!(canonical_kind("po"), "pod");
        assert_eq!(canonical_kind("deploy"), "deployment");
        assert_eq!(canonical_kind("svc"), "service");
        assert_eq!(canonical_kind("pvc"), "persistentvolumeclaim");
        assert_eq!(canonical_kind("widgets"), "widgets"); // unknown passes through
    }

    #[test]
    fn ns_resolution_prefers_override_then_default() {
        let o = Opts {
            kubeconfig: String::new(),
            context: String::new(),
            namespace: "team-a".into(),
        };
        assert_eq!(o.ns(Some("override")), "override");
        assert_eq!(o.ns(None), "team-a");
        let empty = Opts {
            kubeconfig: String::new(),
            context: String::new(),
            namespace: String::new(),
        };
        assert_eq!(empty.ns(None), "default");
    }
}
