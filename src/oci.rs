//! Container images: Docker Hub search/metadata plus generic OCI-registry
//! inspection (tags + manifests) for any registry that implements the OCI
//! Distribution Spec (Docker Hub, GHCR, Quay, self-hosted, …).
//!
//! Golden rules: keyless by default. Docker Hub's `hub.docker.com` JSON API is a
//! plain GET. The distribution endpoints use the spec's **anonymous** bearer-token
//! flow: a public pull triggers a `401` with a `WWW-Authenticate: Bearer realm=…`
//! challenge, we fetch a token from that realm (no credentials) and retry. No
//! login, no stored secrets — exactly how `docker pull` works for public images.

use anyhow::{anyhow, Context, Result};
use reqwest::header::{ACCEPT, WWW_AUTHENTICATE};
use reqwest::{Client, Response, StatusCode};
use serde_json::Value;

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
    // Split off an optional @digest first (a digest also contains ':').
    let (name_tag, digest) = match s.split_once('@') {
        Some((n, d)) => (n, Some(d.to_string())),
        None => (s, None),
    };

    // A leading segment is a registry host if it looks like a domain or has a port.
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

    // The tag is a ':' in the final path segment (the host's port is already gone).
    let (path_no_tag, tag) = match path.rfind(':') {
        Some(idx) if !path[idx + 1..].contains('/') => {
            (path[..idx].to_string(), Some(path[idx + 1..].to_string()))
        }
        _ => (path.clone(), None),
    };

    if path_no_tag.is_empty() {
        return Err(anyhow!("no repository in reference '{input}'"));
    }

    // Docker Hub official images live under the implicit `library/` namespace.
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
    // Registries return the token as `token` (Docker) or `access_token` (OAuth2).
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

/// List tags for any OCI image, via the distribution `/tags/list` endpoint.
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
