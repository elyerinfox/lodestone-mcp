//! gitea.com forge spec (the public Gitea instance). Same blob URL layout as
//! other Gitea hosts:
//!   https://gitea.com/owner/repo/src/branch/<ref>/<path>

use std::sync::LazyLock;

use regex::Regex;

use super::ForgeSpec;

static SRC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://[^/]+/([^/]+)/([^/]+)/src/(?:branch|commit|tag)/[^/]+/(.+)$").unwrap()
});

pub(super) static SPEC: ForgeSpec = ForgeSpec {
    id: "gitea",
    domain: "gitea.com",
    repo_path: extract,
};

fn extract(url: &str) -> Option<(String, String)> {
    let c = SRC_RE.captures(url)?;
    Some((format!("{}/{}", &c[1], &c[2]), c[3].to_string()))
}
