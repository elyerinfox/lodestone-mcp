//! Container-image skills (keyless): Docker Hub search/metadata/tags plus generic
//! OCI-registry inspection (tags + manifests) for any registry implementing the
//! OCI Distribution Spec (Docker Hub, GHCR, Quay, self-hosted).
//!
//! Skills: `docker_search`, `docker_image`, `docker_tags` (Docker Hub JSON API),
//! `oci_tags`, `oci_manifest` (distribution API, anonymous bearer-token flow).
//!
//! Golden rules: keyless by default. Docker Hub's `hub.docker.com` JSON API is a
//! plain GET. The distribution endpoints use the spec's **anonymous** bearer-token
//! flow: a public pull triggers a `401` with a `WWW-Authenticate: Bearer realm=…`
//! challenge, we fetch a token from that realm (no credentials) and retry.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use futures::future::BoxFuture;
use reqwest::header::{ACCEPT, WWW_AUTHENTICATE};
use reqwest::{Client, Response, StatusCode};
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::{human_count, human_size};
use crate::{clamp, internal, invalid, text_result};

/// Accept header advertising every manifest media type we can read (multi-arch
/// index first, then single-image), so a registry returns the richest form.
const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.index.v1+json, \
    application/vnd.docker.distribution.manifest.list.v2+json, \
    application/vnd.oci.image.manifest.v1+json, \
    application/vnd.docker.distribution.manifest.v2+json";

// ---------------------------------------------------------------------------
// Docker Hub (hub.docker.com JSON API — keyless)
// ---------------------------------------------------------------------------

/// Search Docker Hub repositories. Returns the raw JSON (`results` array).
pub async fn hub_search(http: &Client, query: &str, limit: usize) -> Result<Value> {
    let size = limit.clamp(1, 100).to_string();
    Ok(http
        .get("https://hub.docker.com/v2/search/repositories/")
        .query(&[("query", query), ("page_size", size.as_str())])
        .header(ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// Fetch a Docker Hub repository's metadata (`namespace/repo`).
pub async fn hub_repo(http: &Client, namespace: &str, repo: &str) -> Result<Value> {
    Ok(http
        .get(format!(
            "https://hub.docker.com/v2/repositories/{namespace}/{repo}/"
        ))
        .header(ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

/// List a Docker Hub repository's tags (newest first), with size/date/arch.
pub async fn hub_tags(http: &Client, namespace: &str, repo: &str, limit: usize) -> Result<Value> {
    let size = limit.clamp(1, 100).to_string();
    Ok(http
        .get(format!(
            "https://hub.docker.com/v2/repositories/{namespace}/{repo}/tags/"
        ))
        .query(&[("page_size", size.as_str()), ("ordering", "last_updated")])
        .header(ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?)
}

// ---------------------------------------------------------------------------
// Image references
// ---------------------------------------------------------------------------

/// A parsed image reference, normalized for both the Docker Hub JSON API and the
/// OCI distribution API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// Distribution API host, e.g. `registry-1.docker.io`, `ghcr.io`, `quay.io`.
    pub registry_host: String,
    /// Repository path used in `/v2/<repository>/…`, e.g. `library/nginx`.
    pub repository: String,
    /// Tag or digest to inspect (defaults to `latest`).
    pub reference: String,
    /// Whether this resolves to Docker Hub (enables the hub.docker.com helpers).
    pub is_dockerhub: bool,
}

impl ImageRef {
    /// `(namespace, repo)` for the Docker Hub JSON API, when this is a Hub image.
    pub fn hub_namespace_repo(&self) -> Option<(String, String)> {
        if !self.is_dockerhub {
            return None;
        }
        self.repository
            .split_once('/')
            .map(|(ns, repo)| (ns.to_string(), repo.to_string()))
    }

    /// Human-readable `host/repository:reference`.
    pub fn display(&self) -> String {
        format!(
            "{}/{}:{}",
            self.registry_host, self.repository, self.reference
        )
    }
}

/// Parse an image reference like `nginx`, `nginx:1.25`, `bitnami/nginx`,
/// `ghcr.io/owner/image:tag`, or `quay.io/ns/repo@sha256:…`.
pub fn parse_ref(input: &str) -> Result<ImageRef> {
    let s = input.trim();
    if s.is_empty() {
        return Err(anyhow!("empty image reference"));
    }
    let (name_tag, digest) = match s.split_once('@') {
        Some((n, d)) => (n, Some(d.to_string())),
        None => (s, None),
    };

    let (host, path) = match name_tag.split_once('/') {
        Some((first, rest))
            if first.contains('.') || first.contains(':') || first == "localhost" =>
        {
            (first.to_string(), rest.to_string())
        }
        _ => (String::new(), name_tag.to_string()),
    };

    let is_dockerhub = matches!(
        host.as_str(),
        "" | "docker.io" | "index.docker.io" | "registry-1.docker.io"
    );

    let (path_no_tag, tag) = match path.rfind(':') {
        Some(idx) if !path[idx + 1..].contains('/') => {
            (path[..idx].to_string(), Some(path[idx + 1..].to_string()))
        }
        _ => (path.clone(), None),
    };

    if path_no_tag.is_empty() {
        return Err(anyhow!("no repository in reference '{input}'"));
    }

    let repository = if is_dockerhub && !path_no_tag.contains('/') {
        format!("library/{path_no_tag}")
    } else {
        path_no_tag
    };
    let registry_host = if is_dockerhub {
        "registry-1.docker.io".to_string()
    } else {
        host
    };
    let reference = digest.or(tag).unwrap_or_else(|| "latest".to_string());

    Ok(ImageRef {
        registry_host,
        repository,
        reference,
        is_dockerhub,
    })
}

// ---------------------------------------------------------------------------
// OCI distribution API (generic, anonymous token flow)
// ---------------------------------------------------------------------------

/// GET a distribution endpoint, transparently performing the anonymous
/// bearer-token dance if the registry answers `401` with a `Bearer` challenge.
async fn authed_get(http: &Client, url: &str, accept: &str) -> Result<Response> {
    let resp = http.get(url).header(ACCEPT, accept).send().await?;
    if resp.status() != StatusCode::UNAUTHORIZED {
        return Ok(resp);
    }
    let challenge = resp
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("registry requires auth but sent no Bearer challenge"))?;
    let (realm, service, scope) = parse_challenge(&challenge)?;

    let mut params: Vec<(&str, &str)> = Vec::new();
    if let Some(s) = &service {
        params.push(("service", s));
    }
    if let Some(s) = &scope {
        params.push(("scope", s));
    }
    let token: Value = http
        .get(&realm)
        .query(&params)
        .send()
        .await?
        .error_for_status()
        .context("fetching anonymous registry token")?
        .json()
        .await?;
    let token = token
        .get("token")
        .or_else(|| token.get("access_token"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow!("registry token response had no token"))?;

    Ok(http
        .get(url)
        .header(ACCEPT, accept)
        .bearer_auth(token)
        .send()
        .await?)
}

/// Parse a `Bearer realm="…",service="…",scope="…"` challenge into its parts.
fn parse_challenge(header: &str) -> Result<(String, Option<String>, Option<String>)> {
    let rest = header
        .trim()
        .strip_prefix("Bearer ")
        .ok_or_else(|| anyhow!("unsupported auth scheme: {header}"))?;
    let mut realm = None;
    let mut service = None;
    let mut scope = None;
    for part in rest.split(',') {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"').to_string();
        match k.trim() {
            "realm" => realm = Some(v),
            "service" => service = Some(v),
            "scope" => scope = Some(v),
            _ => {}
        }
    }
    let realm = realm.ok_or_else(|| anyhow!("Bearer challenge missing realm"))?;
    Ok((realm, service, scope))
}

/// List tags for any OCI image via the distribution `/tags/list` endpoint.
/// Returns `(repository_name, tags)` truncated to `limit`.
pub async fn list_tags(http: &Client, r: &ImageRef, limit: usize) -> Result<(String, Vec<String>)> {
    let n = limit.clamp(1, 200).to_string();
    let url = format!(
        "https://{}/v2/{}/tags/list?n={n}",
        r.registry_host, r.repository
    );
    let resp = authed_get(http, &url, "application/json")
        .await?
        .error_for_status()
        .with_context(|| format!("listing tags for {}", r.display()))?;
    let v: Value = resp.json().await?;
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or(&r.repository)
        .to_string();
    let mut tags: Vec<String> = v
        .get("tags")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    tags.truncate(limit);
    Ok((name, tags))
}

/// A summarized image manifest (or multi-arch index).
pub struct ManifestInfo {
    pub media_type: String,
    pub digest: Option<String>,
    /// Platforms (`os/arch[/variant]`) for a multi-arch index; empty otherwise.
    pub platforms: Vec<String>,
    /// Layer count + total compressed size for a single-image manifest.
    pub layers: usize,
    pub total_size: u64,
    pub config_digest: Option<String>,
}

/// Fetch and summarize an image's manifest (handles both single and multi-arch).
pub async fn manifest(http: &Client, r: &ImageRef) -> Result<ManifestInfo> {
    let url = format!(
        "https://{}/v2/{}/manifests/{}",
        r.registry_host, r.repository, r.reference
    );
    let resp = authed_get(http, &url, MANIFEST_ACCEPT)
        .await?
        .error_for_status()
        .with_context(|| format!("fetching manifest for {}", r.display()))?;
    let digest = resp
        .headers()
        .get("docker-content-digest")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let v: Value = resp.json().await?;

    let media_type = v
        .get("mediaType")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();

    let mut platforms = Vec::new();
    if let Some(list) = v.get("manifests").and_then(|x| x.as_array()) {
        for m in list {
            let Some(p) = m.get("platform") else { continue };
            let os = p.get("os").and_then(|x| x.as_str()).unwrap_or("");
            let arch = p.get("architecture").and_then(|x| x.as_str()).unwrap_or("");
            if os.is_empty() && arch.is_empty() {
                continue;
            }
            let variant = p.get("variant").and_then(|x| x.as_str()).unwrap_or("");
            let mut s = format!("{os}/{arch}");
            if !variant.is_empty() {
                s.push('/');
                s.push_str(variant);
            }
            platforms.push(s);
        }
    }

    let layers_arr = v.get("layers").and_then(|x| x.as_array());
    let layers = layers_arr.map(|a| a.len()).unwrap_or(0);
    let total_size = layers_arr
        .map(|a| {
            a.iter()
                .filter_map(|l| l.get("size").and_then(|s| s.as_u64()))
                .sum()
        })
        .unwrap_or(0);
    let config_digest = v
        .get("config")
        .and_then(|c| c.get("digest"))
        .and_then(|d| d.as_str())
        .map(str::to_string);

    Ok(ManifestInfo {
        media_type,
        digest,
        platforms,
        layers,
        total_size,
        config_digest,
    })
}

// ---------------------------------------------------------------------------
// Formatters
// ---------------------------------------------------------------------------

fn format_docker_search(query: &str, v: &Value, limit: usize) -> String {
    let mut out = format!("Docker Hub results for \"{query}\":\n");
    let empty = Vec::new();
    let results = v
        .get("results")
        .and_then(|x| x.as_array())
        .unwrap_or(&empty);
    for (i, r) in results.iter().take(limit).enumerate() {
        let name = r.get("repo_name").and_then(|x| x.as_str()).unwrap_or("");
        let official = r
            .get("is_official")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);
        let stars = r.get("star_count").and_then(|x| x.as_i64()).unwrap_or(0);
        let pulls = r.get("pull_count").and_then(|x| x.as_i64()).unwrap_or(0);
        let desc = r
            .get("short_description")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let url = if official {
            format!("https://hub.docker.com/_/{name}")
        } else {
            format!("https://hub.docker.com/r/{name}")
        };
        out.push_str(&format!(
            "\n{}. {name}{}\n   {url}\n   stars {} · pulls {}\n",
            i + 1,
            if official { " [official]" } else { "" },
            human_count(stars),
            human_count(pulls),
        ));
        if !desc.is_empty() {
            out.push_str(&format!("   {desc}\n"));
        }
    }
    out
}

fn format_docker_image(v: &Value, ns: &str, repo: &str) -> String {
    let official = ns == "library";
    let full = if official {
        repo.to_string()
    } else {
        format!("{ns}/{repo}")
    };
    let url = if official {
        format!("https://hub.docker.com/_/{repo}")
    } else {
        format!("https://hub.docker.com/r/{ns}/{repo}")
    };
    let mut out = format!("Docker Hub image: {full}");
    if official {
        out.push_str(" [official]");
    }
    out.push('\n');
    if let Some(d) = v
        .get("description")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        out.push_str(&format!("{d}\n"));
    }
    if let Some(s) = v.get("star_count").and_then(|x| x.as_i64()) {
        out.push_str(&format!("  stars: {}\n", human_count(s)));
    }
    if let Some(p) = v.get("pull_count").and_then(|x| x.as_i64()) {
        out.push_str(&format!("  pulls: {}\n", human_count(p)));
    }
    if let Some(u) = v
        .get("last_updated")
        .and_then(|x| x.as_str())
        .and_then(|d| d.get(..10))
    {
        out.push_str(&format!("  last updated: {u}\n"));
    }
    out.push_str(&format!("  {url}\n"));
    if let Some(full_desc) = v
        .get("full_description")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push('\n');
        out.push_str(&crate::util::truncate_chars(full_desc, 3000));
        out.push('\n');
    }
    out
}

fn format_docker_tags(v: &Value, ns: &str, repo: &str) -> String {
    let full = if ns == "library" {
        repo.to_string()
    } else {
        format!("{ns}/{repo}")
    };
    let empty = Vec::new();
    let results = v
        .get("results")
        .and_then(|x| x.as_array())
        .unwrap_or(&empty);
    let mut out = format!("Tags for {full} ({} shown, newest first):\n", results.len());
    for t in results {
        let name = t.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let size = t.get("full_size").and_then(|x| x.as_u64()).unwrap_or(0);
        let pushed = t
            .get("tag_last_pushed")
            .and_then(|x| x.as_str())
            .and_then(|d| d.get(..10))
            .unwrap_or("");
        let archs: Vec<&str> = t
            .get("images")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|im| im.get("architecture").and_then(|x| x.as_str()))
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        out.push_str(&format!("\n  {name}"));
        let mut facts = Vec::new();
        if size > 0 {
            facts.push(human_size(size));
        }
        if !pushed.is_empty() {
            facts.push(pushed.to_string());
        }
        if !archs.is_empty() {
            facts.push(archs.join("/"));
        }
        if !facts.is_empty() {
            out.push_str(&format!("  ({})", facts.join(" · ")));
        }
    }
    out.push('\n');
    out
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerSearchArgs {
    /// What to search for on Docker Hub (image name, keyword).
    query: String,
    /// Maximum number of results to return. Default 10, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerImageArgs {
    /// A Docker Hub image: `nginx`, `library/nginx`, or `bitnami/redis` (an
    /// optional `:tag` is ignored — this reports the repository).
    image: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DockerTagsArgs {
    /// A Docker Hub image: `nginx`, `library/nginx`, or `grafana/grafana`.
    image: String,
    /// Maximum number of tags to return (newest first). Default 15, capped 50.
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct OciTagsArgs {
    /// An image reference on any OCI registry: `nginx`, `ghcr.io/owner/image`,
    /// `quay.io/ns/repo`, `localhost:5000/team/app`.
    reference: String,
    /// Maximum number of tags to return. Default 30, capped 200.
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct OciManifestArgs {
    /// An image reference (with optional `:tag` or `@sha256:…`) on any OCI
    /// registry, e.g. `nginx:1.27`, `ghcr.io/owner/image:latest`.
    reference: String,
}

pub struct DockerSearch;
impl Skill for DockerSearch {
    fn name(&self) -> &'static str {
        "docker_search"
    }
    fn description(&self) -> &'static str {
        "Search Docker Hub for container images (keyless). Returns name, official/verified status, \
        stars, pull count, and a short description. Use docker_image for one image's details, \
        docker_tags to list its tags."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DockerSearchArgs>()?;
            let limit = clamp(args.max_results, 10, 25);
            let key = format!("docker_search|{limit}|{}", args.query);
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let v = hub_search(&server.http, &args.query, limit)
                .await
                .map_err(internal)?;
            let out = format_docker_search(&args.query, &v, limit);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct DockerImage;
impl Skill for DockerImage {
    fn name(&self) -> &'static str {
        "docker_image"
    }
    fn description(&self) -> &'static str {
        "Get a Docker Hub repository's details (keyless): description, stars, pull count, \
        last-updated date, and the long description. Accepts `nginx`, `library/nginx`, or `org/image`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerImageArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DockerImageArgs>()?;
            let r = parse_ref(&args.image).map_err(|e| invalid(e.to_string()))?;
            let (ns, repo) = r.hub_namespace_repo().ok_or_else(|| {
                invalid(format!(
                    "'{}' is not a Docker Hub image; use oci_manifest for other registries",
                    args.image
                ))
            })?;
            let key = format!("docker_image|{ns}/{repo}");
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let v = hub_repo(&server.http, &ns, &repo).await.map_err(internal)?;
            let out = format_docker_image(&v, &ns, &repo);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct DockerTags;
impl Skill for DockerTags {
    fn name(&self) -> &'static str {
        "docker_tags"
    }
    fn description(&self) -> &'static str {
        "List a Docker Hub image's tags (keyless), newest first, with compressed size, last-pushed \
        date, and architectures. Accepts `nginx`, `library/nginx`, or `org/image`. For \
        non-Docker-Hub registries use oci_tags."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DockerTagsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DockerTagsArgs>()?;
            let limit = clamp(args.max_results, 15, 50);
            let r = parse_ref(&args.image).map_err(|e| invalid(e.to_string()))?;
            let (ns, repo) = r.hub_namespace_repo().ok_or_else(|| {
                invalid(format!(
                    "'{}' is not a Docker Hub image; use oci_tags for other registries",
                    args.image
                ))
            })?;
            let key = format!("docker_tags|{limit}|{ns}/{repo}");
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let v = hub_tags(&server.http, &ns, &repo, limit)
                .await
                .map_err(internal)?;
            let out = format_docker_tags(&v, &ns, &repo);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct OciTags;
impl Skill for OciTags {
    fn name(&self) -> &'static str {
        "oci_tags"
    }
    fn description(&self) -> &'static str {
        "List tags for an image on ANY OCI registry (keyless, anonymous pull): Docker Hub, GHCR \
        (ghcr.io), Quay (quay.io), or a self-hosted registry. Accepts `nginx`, \
        `ghcr.io/owner/image`, `quay.io/ns/repo`. Use oci_manifest to inspect one tag's platforms."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<OciTagsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<OciTagsArgs>()?;
            let limit = clamp(args.max_results, 30, 200);
            let r = parse_ref(&args.reference).map_err(|e| invalid(e.to_string()))?;
            let key = format!("oci_tags|{limit}|{}/{}", r.registry_host, r.repository);
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let (name, tags) = list_tags(&server.http, &r, limit).await.map_err(internal)?;
            if tags.is_empty() {
                return Ok(text_result(format!("No tags found for {}.", r.display())));
            }
            let out = format!(
                "Tags for {}/{name} ({} shown):\n{}",
                r.registry_host,
                tags.len(),
                tags.iter()
                    .map(|t| format!("  {t}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct OciManifest;
impl Skill for OciManifest {
    fn name(&self) -> &'static str {
        "oci_manifest"
    }
    fn description(&self) -> &'static str {
        "Inspect an image's manifest on ANY OCI registry (keyless, anonymous pull). For a \
        multi-arch image, lists the platforms (os/arch); for a single image, the layer count, total \
        compressed size, and config digest. Accepts `nginx:1.27`, `ghcr.io/owner/image@sha256:…`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<OciManifestArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<OciManifestArgs>()?;
            let r = parse_ref(&args.reference).map_err(|e| invalid(e.to_string()))?;
            let key = format!("oci_manifest|{}", r.display());
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let m = manifest(&server.http, &r).await.map_err(internal)?;
            let mut out = format!(
                "Manifest for {}\n  media type: {}",
                r.display(),
                m.media_type
            );
            if let Some(d) = &m.digest {
                out.push_str(&format!("\n  digest: {d}"));
            }
            if !m.platforms.is_empty() {
                out.push_str(&format!(
                    "\n  multi-arch ({} platforms): {}",
                    m.platforms.len(),
                    m.platforms.join(", ")
                ));
            } else {
                out.push_str(&format!(
                    "\n  layers: {} ({})",
                    m.layers,
                    human_size(m.total_size)
                ));
                if let Some(c) = &m.config_digest {
                    out.push_str(&format!("\n  config: {c}"));
                }
            }
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
#[cfg(test)]
mod live {
    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent("lodestone-mcp/0.1.0 (+https://github.com/elyerinfox/lodestone-mcp)")
            .build()
            .unwrap()
    }

    /// Docker Hub search v1 search/repositories — the legacy keyless endpoint.
    #[tokio::test]
    #[ignore]
    async fn docker_hub_search_live() {
        let r = http()
            .get("https://hub.docker.com/v2/search/repositories/?query=nginx&page_size=3")
            .send().await.expect("network").error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        let results = v["results"].as_array().expect("missing results");
        assert!(!results.is_empty());
        for k in ["repo_name", "star_count", "pull_count"] {
            assert!(results[0].get(k).is_some(), "missing field {k}");
        }
    }

    /// Docker Hub repo metadata for `library/nginx`.
    #[tokio::test]
    #[ignore]
    async fn docker_hub_image_live() {
        let r = http()
            .get("https://hub.docker.com/v2/repositories/library/nginx/")
            .send().await.expect("network").error_for_status().unwrap();
        let v: serde_json::Value = r.json().await.unwrap();
        for k in ["name", "namespace", "pull_count", "star_count"] {
            assert!(v.get(k).is_some(), "missing field {k}");
        }
    }

    /// GHCR is an OCI registry — anonymous tag list for a public image.
    #[tokio::test]
    #[ignore]
    async fn ghcr_tags_anonymous_live() {
        // Need a bearer token for GHCR even anonymous; the docker_v2 flow asks
        // for it via WWW-Authenticate. Mirror that two-step.
        let auth = http()
            .get("https://ghcr.io/token?scope=repository:nginxinc/nginx-unprivileged:pull")
            .send().await.expect("network");
        if !auth.status().is_success() {
            eprintln!("skipping ghcr: token endpoint {}", auth.status());
            return;
        }
        let tv: serde_json::Value = auth.json().await.unwrap();
        let Some(tok) = tv.get("token").and_then(|x| x.as_str()) else {
            eprintln!("skipping ghcr: no token field");
            return;
        };
        let r = http()
            .get("https://ghcr.io/v2/nginxinc/nginx-unprivileged/tags/list")
            .bearer_auth(tok)
            .send().await.expect("network");
        if !r.status().is_success() {
            eprintln!("skipping ghcr tag list: {}", r.status());
            return;
        }
        let v: serde_json::Value = r.json().await.unwrap();
        assert!(v["tags"].is_array(), "missing tags array");
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(DockerSearch),
        Box::new(DockerImage),
        Box::new(DockerTags),
        Box::new(OciTags),
        Box::new(OciManifest),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_official_short_name() {
        let r = parse_ref("nginx").unwrap();
        assert!(r.is_dockerhub);
        assert_eq!(r.registry_host, "registry-1.docker.io");
        assert_eq!(r.repository, "library/nginx");
        assert_eq!(r.reference, "latest");
        assert_eq!(
            r.hub_namespace_repo().unwrap(),
            ("library".into(), "nginx".into())
        );
    }

    #[test]
    fn parse_namespaced_with_tag() {
        let r = parse_ref("bitnami/nginx:1.25").unwrap();
        assert!(r.is_dockerhub);
        assert_eq!(r.repository, "bitnami/nginx");
        assert_eq!(r.reference, "1.25");
    }

    #[test]
    fn parse_ghcr_with_port_and_digest() {
        let r = parse_ref("ghcr.io/owner/image@sha256:abc123").unwrap();
        assert!(!r.is_dockerhub);
        assert_eq!(r.registry_host, "ghcr.io");
        assert_eq!(r.repository, "owner/image");
        assert_eq!(r.reference, "sha256:abc123");
        assert!(r.hub_namespace_repo().is_none());

        let r = parse_ref("localhost:5000/team/app:dev").unwrap();
        assert_eq!(r.registry_host, "localhost:5000");
        assert_eq!(r.repository, "team/app");
        assert_eq!(r.reference, "dev");
    }

    #[test]
    fn parse_challenge_extracts_parts() {
        let (realm, service, scope) = parse_challenge(
            r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/nginx:pull""#,
        )
        .unwrap();
        assert_eq!(realm, "https://auth.docker.io/token");
        assert_eq!(service.as_deref(), Some("registry.docker.io"));
        assert_eq!(scope.as_deref(), Some("repository:library/nginx:pull"));
    }
}
