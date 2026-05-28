//! GitLab forge spec. Blob URLs look like:
//!   https://gitlab.com/group/subgroup/project/-/blob/<ref>/<path>

use std::sync::LazyLock;

use regex::Regex;

use super::ForgeSpec;

static BLOB_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://[^/]+/(.+?)/-/blob/[^/]+/(.+)$").unwrap());

pub(super) static SPEC: ForgeSpec = ForgeSpec {
    id: "gitlab",
    domain: "gitlab.com",
    repo_path: extract,
};

fn extract(url: &str) -> Option<(String, String)> {
    let c = BLOB_RE.captures(url)?;
    Some((c[1].to_string(), c[2].to_string()))
}
