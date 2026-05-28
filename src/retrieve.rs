//! Retrieval helpers — fetching the full contents of a specific resource once
//! it's been located (a GitHub file, an arbitrary page, a Q&A thread). These
//! are deliberately *not* providers: they retrieve one known thing rather than
//! ranking many candidates.

use std::sync::LazyLock;

use anyhow::{anyhow, Result};
use regex::Regex;
use reqwest::Client;

use crate::util::{html_to_text, truncate_chars};

// ---------------------------------------------------------------------------
// Raw file fetch across forges (no token):
//   GitHub : github.com/.../blob/<ref>/<path>  → raw.githubusercontent.com/.../<ref>/<path>
//   GitLab : <host>/.../-/blob/<ref>/<path>     → <host>/.../-/raw/<ref>/<path>
//   Gitea  : <host>/o/r/src/branch/<ref>/<path> → <host>/o/r/raw/branch/<ref>/<path>
// ---------------------------------------------------------------------------

static GH_BLOB_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://github\.com/([^/]+)/([^/]+)/(?:blob|raw)/([^/]+)/(.+)$").unwrap()
});
static GITEA_SRC_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/src/(branch|commit|tag)/").unwrap());
static SHORTHAND_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^/\s]+)/([^/\s]+)/(.+)$").unwrap());
static LINE_FRAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#L(\d+)(?:[-C]+L?(\d+))?$").unwrap());

/// Resolved raw download target(s) plus any `#L..` line range from the input.
pub struct RawTarget {
    pub candidates: Vec<String>,
    pub line_range: Option<(usize, usize)>,
}

/// Resolve a GitHub/GitLab/Gitea blob (or raw) URL — or a GitHub
/// `owner/repo/path` shorthand — into raw download target(s).
pub fn resolve_raw_file(input: &str) -> Result<RawTarget> {
    let input = input.trim();
    let (base, line_range) = split_line_fragment(input);
    let single = |url: String| {
        Ok(RawTarget {
            candidates: vec![url],
            line_range,
        })
    };

    // Already-raw passthroughs (GitHub raw host, GitLab `/-/raw/`, Gitea `/raw/<ref-kind>/`).
    if base.starts_with("https://raw.githubusercontent.com/")
        || base.starts_with("http://raw.githubusercontent.com/")
        || base.contains("/-/raw/")
        || base.contains("/raw/branch/")
        || base.contains("/raw/commit/")
        || base.contains("/raw/tag/")
    {
        return single(base.to_string());
    }

    // GitHub blob/raw → raw.githubusercontent.com.
    if let Some(c) = GH_BLOB_RE.captures(base) {
        return single(format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            &c[1], &c[2], &c[3], &c[4]
        ));
    }

    // GitLab blob → raw (same host).
    if base.contains("/-/blob/") {
        return single(base.replacen("/-/blob/", "/-/raw/", 1));
    }

    // Gitea/Codeberg blob (`/src/branch|commit|tag/`) → raw (same host).
    if GITEA_SRC_RE.is_match(base) {
        return single(GITEA_SRC_RE.replace(base, "/raw/$1/").into_owned());
    }

    // `owner/repo/path` shorthand → GitHub (try the common default branches).
    if !base.contains("://") {
        if let Some(c) = SHORTHAND_RE.captures(base) {
            let (owner, repo, path) = (&c[1], &c[2], &c[3]);
            let candidates = ["main", "master"]
                .iter()
                .map(|r| format!("https://raw.githubusercontent.com/{owner}/{repo}/{r}/{path}"))
                .collect();
            return Ok(RawTarget {
                candidates,
                line_range,
            });
        }
    }

    Err(anyhow!(
        "could not parse '{input}' as a GitHub/GitLab/Gitea file URL or an 'owner/repo/path' reference"
    ))
}

fn split_line_fragment(input: &str) -> (&str, Option<(usize, usize)>) {
    if let Some(c) = LINE_FRAG_RE.captures(input) {
        let whole = c.get(0).unwrap();
        let start: usize = c[1].parse().unwrap_or(0);
        let end: usize = c
            .get(2)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(start);
        if start > 0 {
            return (&input[..whole.start()], Some((start, end.max(start))));
        }
    }
    (input, None)
}

/// Extract `(owner/repo, branch, path)` from a github.com blob/raw URL.
/// Used by code providers to enrich search hits.
pub fn github_repo_path(url: &str) -> Option<(String, String, String)> {
    GH_BLOB_RE.captures(url).map(|c| {
        (
            format!("{}/{}", &c[1], &c[2]),
            c[3].to_string(),
            c[4].to_string(),
        )
    })
}

/// GET a URL, returning `(body, status)`.
pub async fn fetch_text(client: &Client, url: &str) -> Result<(String, reqwest::StatusCode)> {
    let resp = client.get(url).send().await?;
    let status = resp.status();
    let body = resp.text().await?;
    Ok((body, status))
}

// ---------------------------------------------------------------------------
// Generic readable page fetch (HTML -> text)
// ---------------------------------------------------------------------------

pub async fn fetch_readable(client: &Client, url: &str, max_chars: usize) -> Result<String> {
    let resp = client
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml,*/*")
        .send()
        .await?
        .error_for_status()?;
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.text().await?;
    let text = if ctype.contains("html") || body.trim_start().starts_with('<') {
        html_to_text(&body)
    } else {
        body
    };
    Ok(truncate_chars(&text, max_chars))
}

// ---------------------------------------------------------------------------
// Wayback Machine (Internet Archive — keyless)
// ---------------------------------------------------------------------------

static WAYBACK_TS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/web/(\d+)(?:[a-z_]+)?/").unwrap());

/// Look up the closest archived snapshot for `url` (optionally near `timestamp`,
/// a `YYYYMMDD[hhmmss]` string). Returns a direct, toolbar-free snapshot URL.
pub async fn wayback_snapshot(
    client: &Client,
    url: &str,
    timestamp: Option<&str>,
) -> Result<Option<String>> {
    let mut params = vec![("url", url)];
    if let Some(ts) = timestamp {
        params.push(("timestamp", ts));
    }
    let v: serde_json::Value = client
        .get("https://archive.org/wayback/available")
        .query(&params)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let available = v
        .pointer("/archived_snapshots/closest/available")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    if !available {
        return Ok(None);
    }
    let snapshot = v
        .pointer("/archived_snapshots/closest/url")
        .and_then(|x| x.as_str())
        .map(to_raw_snapshot);
    Ok(snapshot)
}

/// Resolve a snapshot for `url` and return `(snapshot_url, readable_text)`.
pub async fn wayback_fetch(
    client: &Client,
    url: &str,
    timestamp: Option<&str>,
    max_chars: usize,
) -> Result<(String, String)> {
    let snapshot = wayback_snapshot(client, url, timestamp)
        .await?
        .ok_or_else(|| anyhow!("no archived snapshot found for {url}"))?;
    let text = fetch_readable(client, &snapshot, max_chars).await?;
    Ok((snapshot, text))
}

/// Turn a Wayback viewer URL into the raw-content form (insert `id_` after the
/// timestamp so the archive toolbar/rewriting is omitted) over HTTPS.
fn to_raw_snapshot(url: &str) -> String {
    let url = WAYBACK_TS_RE.replace(url, "/web/${1}id_/");
    if let Some(rest) = url.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        url.into_owned()
    }
}

// ---------------------------------------------------------------------------
// StackExchange thread retrieval (keyless public API)
// ---------------------------------------------------------------------------

static QUESTION_ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/questions/(\d+)").unwrap());

pub fn extract_question_id(input: &str) -> Option<String> {
    let t = input.trim();
    if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) {
        return Some(t.to_string());
    }
    QUESTION_ID_RE.captures(t).map(|c| c[1].to_string())
}

/// Fetch `(question_json, answers_json)` for a question id. An optional API key
/// (empty = none) raises the per-IP quota.
pub async fn se_answers(
    client: &Client,
    question_id: &str,
    site: &str,
    max: usize,
    key: &str,
) -> Result<(serde_json::Value, serde_json::Value)> {
    let mut q_params = vec![("site", site), ("filter", "withbody")];
    if !key.is_empty() {
        q_params.push(("key", key));
    }
    let question = client
        .get(format!(
            "https://api.stackexchange.com/2.3/questions/{question_id}"
        ))
        .query(&q_params)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let pagesize = max.clamp(1, 30).to_string();
    let mut a_params = vec![
        ("order", "desc"),
        ("sort", "votes"),
        ("site", site),
        ("filter", "withbody"),
        ("pagesize", pagesize.as_str()),
    ];
    if !key.is_empty() {
        a_params.push(("key", key));
    }
    let answers = client
        .get(format!(
            "https://api.stackexchange.com/2.3/questions/{question_id}/answers"
        ))
        .query(&a_params)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok((question, answers))
}
