//! Codeberg forge spec (a Gitea instance). Blob URLs look like:
//!   https://codeberg.org/owner/repo/src/branch/<ref>/<path>

use std::sync::LazyLock;

use regex::Regex;

use super::ForgeSpec;

static SRC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://[^/]+/([^/]+)/([^/]+)/src/(?:branch|commit|tag)/[^/]+/(.+)$").unwrap()
});

pub(super) static SPEC: ForgeSpec = ForgeSpec {
    id: "codeberg",
    domain: "codeberg.org",
    repo_path: extract,
};

fn extract(url: &str) -> Option<(String, String)> {
    let c = SRC_RE.captures(url)?;
    Some((format!("{}/{}", &c[1], &c[2]), c[3].to_string()))
}
