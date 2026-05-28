//! Forge code-search providers.
//!
//! Every forge shares the SAME underlying logic — a site-scoped web search
//! (scrape-first: DuckDuckGo, then Mojeek; render only if the caller asked) and
//! differs only in DECLARATIVE specifics captured by [`ForgeSpec`]: the id, the
//! domain, and how to parse a blob URL into `(repo, path)`. Adding a forge is one
//! small file (see `gitlab.rs`, `codeberg.rs`, …) that defines a `ForgeSpec`; the
//! shared [`ForgeCodeProvider`] (or [`search`] directly) turns it into results.
//!
//! GitHub is not a forge *provider* here — it's the composite `github` provider
//! (`providers/github.rs`), which reuses [`search`] for its keyless path. Its URL
//! layout is still recognized by [`repo_path`] (via `retrieve::github_repo_path`).

mod codeberg;
mod gitea;
mod gitlab;

use std::sync::LazyLock;

use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use reqwest::Client;

use super::engine;
use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};

static GH_BLOB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://github\.com/([^/]+)/([^/]+)/(?:blob|raw)/([^/]+)/(.+)$").unwrap()
});

/// Extract `(owner/repo, branch, path)` from a github.com blob/raw URL. GitHub's
/// URL layout, recognized here so code providers can enrich search hits and the
/// retrieval skill can resolve a raw download.
pub(crate) fn github_repo_path(url: &str) -> Option<(String, String, String)> {
    GH_BLOB_RE.captures(url).map(|c| {
        (
            format!("{}/{}", &c[1], &c[2]),
            c[3].to_string(),
            c[4].to_string(),
        )
    })
}

/// Declarative description of a code forge.
pub(super) struct ForgeSpec {
    /// Provider id / result attribution.
    pub id: &'static str,
    /// Domain used to scope the web search and to filter results.
    pub domain: &'static str,
    /// Extract `(owner/repo, path)` from one of this forge's blob URLs.
    pub repo_path: fn(&str) -> Option<(String, String)>,
}

static SPECS: &[&ForgeSpec] = &[&gitlab::SPEC, &codeberg::SPEC, &gitea::SPEC];

/// Best-effort `(repo, path)` across known forge URL layouts: GitHub (via
/// `retrieve::github_repo_path`) plus the GitLab/Gitea specs.
pub(super) fn repo_path(url: &str) -> Option<(String, String)> {
    if let Some((repo, _branch, path)) = github_repo_path(url) {
        return Some((repo, path));
    }
    SPECS.iter().find_map(|spec| (spec.repo_path)(url))
}

/// Construct a forge provider by id, if known.
pub(super) fn make(id: &str) -> Option<ForgeCodeProvider> {
    let spec = match id {
        "gitlab" => &gitlab::SPEC,
        "codeberg" => &codeberg::SPEC,
        "gitea" => &gitea::SPEC,
        _ => return None,
    };
    Some(ForgeCodeProvider { spec })
}

/// Build a provider for a user-configured self-hosted forge. `kind` selects the
/// blob-URL layout ("gitlab", or "gitea"/"codeberg"); `domain` is the host to
/// scope the search to. The id/domain/spec are leaked to `'static` (config
/// forges live for the whole process), reusing the existing layout parsers —
/// which are host-agnostic regexes, so they work for any domain.
pub(super) fn make_configured(id: &str, kind: &str, domain: &str) -> Option<ForgeCodeProvider> {
    if id.is_empty() || domain.is_empty() {
        tracing::warn!(id, "configured forge missing a domain; skipping");
        return None;
    }
    let repo_path = match kind.trim().to_ascii_lowercase().as_str() {
        "gitlab" => gitlab::SPEC.repo_path,
        "gitea" | "codeberg" => gitea::SPEC.repo_path,
        other => {
            tracing::warn!(
                id,
                kind = other,
                "unknown forge kind (use \"gitlab\" or \"gitea\"); skipping"
            );
            return None;
        }
    };
    let spec: &'static ForgeSpec = Box::leak(Box::new(ForgeSpec {
        id: Box::leak(id.to_string().into_boxed_str()),
        domain: Box::leak(domain.to_string().into_boxed_str()),
        repo_path,
    }));
    Some(ForgeCodeProvider { spec })
}

/// Shared site-scoped code search for one forge. Scrape-first (DuckDuckGo, then
/// Mojeek); `render` is honored only when the caller requested it.
pub(super) async fn search(
    spec: &ForgeSpec,
    http: &Client,
    query: &SearchQuery,
) -> Result<Vec<SearchResult>> {
    let mut terms = query.text.clone();
    if let Some(lang) = &query.language {
        terms.push(' ');
        terms.push_str(lang);
    }

    // DuckDuckGo honors `site:`; fall back to Mojeek (keyword-scoped) on empty.
    let ddg_query = format!("site:{} {terms}", spec.domain);
    let mut hits = engine::duckduckgo_search(http, &ddg_query, query.render, query.limit * 2)
        .await
        .unwrap_or_default();
    if hits.is_empty() {
        let mojeek_query = format!("{terms} {}", spec.domain);
        hits = engine::mojeek_search(http, &mojeek_query, query.render, query.limit * 3)
            .await
            .unwrap_or_default();
    }

    let out = hits
        .into_iter()
        .filter(|h| h.url.contains(spec.domain))
        .map(|mut h| {
            if let Some((repo, path)) = (spec.repo_path)(&h.url) {
                h.title = format!("{repo} — {path}");
                h.repo = Some(repo);
                h.path = Some(path);
            }
            h
        })
        .take(query.limit)
        .collect();
    Ok(out)
}

/// A forge exposed as a standalone code provider.
pub(super) struct ForgeCodeProvider {
    spec: &'static ForgeSpec,
}

#[async_trait]
impl SearchProvider for ForgeCodeProvider {
    fn id(&self) -> &'static str {
        self.spec.id
    }
    fn kind(&self) -> ProviderKind {
        ProviderKind::Code
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        search(self.spec, http, query).await
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn gitlab_blob_url_parses_repo_and_path() {
        let (repo, path) = (super::gitlab::SPEC.repo_path)(
            "https://gitlab.com/group/sub/proj/-/blob/main/src/lib.rs",
        )
        .unwrap();
        assert_eq!(repo, "group/sub/proj");
        assert_eq!(path, "src/lib.rs");
    }

    #[test]
    fn gitea_and_codeberg_src_urls_parse() {
        let (repo, path) = (super::gitea::SPEC.repo_path)(
            "https://gitea.com/owner/repo/src/branch/main/cmd/main.go",
        )
        .unwrap();
        assert_eq!(repo, "owner/repo");
        assert_eq!(path, "cmd/main.go");

        let (repo, path) = (super::codeberg::SPEC.repo_path)(
            "https://codeberg.org/owner/repo/src/commit/abc123/a/b.py",
        )
        .unwrap();
        assert_eq!(repo, "owner/repo");
        assert_eq!(path, "a/b.py");
    }

    #[test]
    fn repo_path_recognizes_github_and_rejects_unknown() {
        let (repo, path) =
            super::repo_path("https://github.com/rust-lang/rust/blob/master/README.md").unwrap();
        assert_eq!(repo, "rust-lang/rust");
        assert_eq!(path, "README.md");

        assert!(super::repo_path("https://example.com/not/a/forge/page").is_none());
    }

    #[test]
    fn configured_forge_uses_layout_and_domain() {
        // Gitea layout on a self-hosted host.
        let p = super::make_configured("myhost", "gitea", "git.example.com").unwrap();
        assert_eq!(p.spec.id, "myhost");
        assert_eq!(p.spec.domain, "git.example.com");
        let (repo, path) =
            (p.spec.repo_path)("https://git.example.com/o/r/src/branch/main/a.rs").unwrap();
        assert_eq!(repo, "o/r");
        assert_eq!(path, "a.rs");

        // GitLab layout maps to the /-/blob/ parser.
        let g = super::make_configured("priv-gl", "gitlab", "gl.example.com").unwrap();
        let (repo, path) =
            (g.spec.repo_path)("https://gl.example.com/grp/proj/-/blob/main/x.rs").unwrap();
        assert_eq!(repo, "grp/proj");
        assert_eq!(path, "x.rs");

        // Unknown kind / missing domain are rejected.
        assert!(super::make_configured("x", "svn", "h.example.com").is_none());
        assert!(super::make_configured("x", "gitea", "").is_none());
    }
}
