//! Forge code-search providers.
//!
//! Every forge shares the SAME underlying logic — a site-scoped web search
//! (DuckDuckGo with a Mojeek fallback, render-aware) filtered to the forge's
//! domain — and differs only in DECLARATIVE specifics captured by [`ForgeSpec`]:
//! the id, the domain, and how to parse a blob URL into `(repo, path)`. Adding a
//! forge is therefore one small file (see `gitlab.rs`, `codeberg.rs`, …) that
//! defines a `ForgeSpec`; the shared [`ForgeCodeProvider`] turns it into a
//! working provider.

mod codeberg;
mod gitea;
mod github;
mod gitlab;

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use super::{duckduckgo, mojeek};
use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};

/// Declarative description of a code forge.
pub(super) struct ForgeSpec {
    /// Provider id, as used in config and result attribution.
    pub id: &'static str,
    /// Domain used to scope the web search and to filter results.
    pub domain: &'static str,
    /// Extract `(owner/repo, path)` from one of this forge's blob URLs.
    pub repo_path: fn(&str) -> Option<(String, String)>,
}

/// All known forge specs — also used to enrich generic web/code hits.
static SPECS: &[&ForgeSpec] = &[&github::SPEC, &gitlab::SPEC, &codeberg::SPEC, &gitea::SPEC];

/// Best-effort `(repo, path)` extraction across all known forge URL layouts.
pub(super) fn repo_path(url: &str) -> Option<(String, String)> {
    SPECS.iter().find_map(|spec| (spec.repo_path)(url))
}

/// Construct a forge provider by id, if known.
pub(super) fn make(id: &str) -> Option<ForgeCodeProvider> {
    let spec = match id {
        "github_web" => &github::SPEC,
        "gitlab" => &gitlab::SPEC,
        "codeberg" => &codeberg::SPEC,
        "gitea" => &gitea::SPEC,
        _ => return None,
    };
    Some(ForgeCodeProvider { spec })
}

/// Shared site-scoped code search for a single forge, driven by its [`ForgeSpec`].
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
        let mut terms = query.text.clone();
        if let Some(lang) = &query.language {
            terms.push(' ');
            terms.push_str(lang);
        }

        // DuckDuckGo honors `site:`; fall back to Mojeek (keyword-scoped) when
        // DDG returns nothing.
        let ddg_query = format!("site:{} {terms}", self.spec.domain);
        let mut hits = duckduckgo::search_raw(http, &ddg_query, query.render, query.limit * 2)
            .await
            .unwrap_or_default();
        if hits.is_empty() {
            let mojeek_query = format!("{terms} {}", self.spec.domain);
            hits = mojeek::search_raw(http, &mojeek_query, query.render, query.limit * 3)
                .await
                .unwrap_or_default();
        }

        let out = hits
            .into_iter()
            .filter(|h| h.url.contains(self.spec.domain))
            .map(|mut h| {
                if let Some((repo, path)) = (self.spec.repo_path)(&h.url) {
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
}
