//! GitHub forge spec — keyless web scrape (id `github_web`). The token-based
//! GitHub code-search API is the separate `github` provider.

use super::ForgeSpec;
use crate::retrieve::github_repo_path;

pub(super) static SPEC: ForgeSpec = ForgeSpec {
    id: "github_web",
    domain: "github.com",
    repo_path: extract,
};

fn extract(url: &str) -> Option<(String, String)> {
    github_repo_path(url).map(|(repo, _branch, path)| (repo, path))
}
