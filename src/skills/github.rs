//! GitHub metadata skills — `github_releases`, `github_user`, `github_repo`.
//! Keyless (GitHub's REST API allows unauthenticated reads); an optional
//! `[github].token` raises the rate limit. Accept `owner/repo` or a github.com URL.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::{indent, truncate_chars};
use crate::{clamp, internal, invalid, text_result};

// ---------------------------------------------------------------------------
// GitHub reference parsing + REST API (keyless; optional token raises the limit)
// ---------------------------------------------------------------------------

/// Parse a GitHub `owner/repo` from a shorthand (`owner/repo`, optionally with
/// more path) or a github.com URL. Returns `None` if it can't.
pub fn github_owner_repo(input: &str) -> Option<String> {
    let mut s = input.trim();
    for prefix in ["https://", "http://", "www.", "github.com/"] {
        s = s.strip_prefix(prefix).unwrap_or(s);
    }
    let s = s.trim_start_matches('/');
    let mut parts = s.split('/').filter(|p| !p.is_empty());
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim().trim_end_matches(".git");
    if owner.is_empty() || repo.is_empty() || owner.contains(' ') || repo.contains(' ') {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Parse a GitHub username/org login from a bare login (`rust-lang`), an `@login`,
/// or a github.com URL. Returns the first path segment.
pub fn github_user_login(input: &str) -> Option<String> {
    let mut s = input.trim().trim_start_matches('@');
    for prefix in ["https://", "http://", "www.", "github.com/"] {
        s = s.strip_prefix(prefix).unwrap_or(s);
    }
    let login = s.trim_start_matches('/').split('/').next()?.trim();
    if login.is_empty() || login.contains(' ') {
        return None;
    }
    Some(login.to_string())
}

/// GET a GitHub REST API path (e.g. `/users/rust-lang`) and return the JSON.
/// Keyless; an optional token (empty = none) raises the rate limit.
async fn github_api(
    client: &Client,
    path: &str,
    token: &str,
    query: &[(&str, &str)],
) -> Result<Value> {
    let mut req = client
        .get(format!("https://api.github.com{path}"))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if !query.is_empty() {
        req = req.query(query);
    }
    if !token.is_empty() {
        req = req.bearer_auth(token);
    }
    Ok(req.send().await?.error_for_status()?.json().await?)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GithubReleasesArgs {
    /// A GitHub repo as `owner/repo` or a github.com URL.
    repo: String,
    /// Max releases to return (newest first). Default 5, capped 30.
    #[serde(default)]
    max_results: Option<u32>,
    /// Include pre-releases and drafts (default false = stable releases only).
    #[serde(default)]
    include_prereleases: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GithubUserArgs {
    /// A GitHub username or org login (e.g. `rust-lang`, `@octocat`, or a
    /// github.com/<user> URL).
    user: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GithubRepoArgs {
    /// A GitHub repo as `owner/repo` or a github.com URL.
    repo: String,
}

fn format_user(v: &Value, fallback: &str) -> String {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).filter(|x| !x.is_empty());
    let n = |k: &str| v.get(k).and_then(|x| x.as_i64());
    let login = s("login").unwrap_or(fallback);
    let kind = s("type").unwrap_or("User");
    let mut out = format!("{kind}: {login}");
    if let Some(name) = s("name") {
        out.push_str(&format!(" ({name})"));
    }
    out.push('\n');
    if let Some(bio) = s("bio") {
        out.push_str(&format!("{bio}\n"));
    }
    let mut facts = Vec::new();
    if let Some(c) = s("company") {
        facts.push(format!("company: {c}"));
    }
    if let Some(l) = s("location") {
        facts.push(format!("location: {l}"));
    }
    if let Some(blog) = s("blog") {
        facts.push(format!("blog: {blog}"));
    }
    if let Some(e) = s("email") {
        facts.push(format!("email: {e}"));
    }
    if let Some(r) = n("public_repos") {
        facts.push(format!("public repos: {r}"));
    }
    if let Some(f) = n("followers") {
        facts.push(format!("followers: {f}"));
    }
    if let Some(f) = n("following") {
        facts.push(format!("following: {f}"));
    }
    if let Some(joined) = s("created_at").and_then(|d| d.get(..10)) {
        facts.push(format!("joined: {joined}"));
    }
    for f in facts {
        out.push_str(&format!("  {f}\n"));
    }
    if let Some(u) = s("html_url") {
        out.push_str(&format!("  {u}\n"));
    }
    out
}

fn format_repo(v: &Value, fallback: &str) -> String {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).filter(|x| !x.is_empty());
    let n = |k: &str| v.get(k).and_then(|x| x.as_i64());
    let flag = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    let full = s("full_name").unwrap_or(fallback);
    let mut out = full.to_string();
    if flag("archived") {
        out.push_str(" [archived]");
    }
    if flag("fork") {
        out.push_str(" [fork]");
    }
    out.push('\n');
    if let Some(d) = s("description") {
        out.push_str(&format!("{d}\n"));
    }
    let mut facts = Vec::new();
    if let Some(x) = n("stargazers_count") {
        facts.push(format!("stars: {x}"));
    }
    if let Some(x) = n("forks_count") {
        facts.push(format!("forks: {x}"));
    }
    if let Some(x) = n("open_issues_count") {
        facts.push(format!("open issues: {x}"));
    }
    if let Some(lang) = s("language") {
        facts.push(format!("language: {lang}"));
    }
    if let Some(topics) = v.get("topics").and_then(|x| x.as_array()) {
        let t: Vec<&str> = topics.iter().filter_map(|x| x.as_str()).collect();
        if !t.is_empty() {
            facts.push(format!("topics: {}", t.join(", ")));
        }
    }
    if let Some(lic) = v
        .get("license")
        .and_then(|l| l.get("spdx_id"))
        .and_then(|x| x.as_str())
        .filter(|x| !x.is_empty() && *x != "NOASSERTION")
    {
        facts.push(format!("license: {lic}"));
    }
    if let Some(db) = s("default_branch") {
        facts.push(format!("default branch: {db}"));
    }
    if let Some(hp) = s("homepage") {
        facts.push(format!("homepage: {hp}"));
    }
    if let Some(pa) = s("pushed_at").and_then(|d| d.get(..10)) {
        facts.push(format!("last push: {pa}"));
    }
    for f in facts {
        out.push_str(&format!("  {f}\n"));
    }
    if let Some(u) = s("html_url") {
        out.push_str(&format!("  {u}\n"));
    }
    out
}

pub struct GithubReleases;
impl Skill for GithubReleases {
    fn name(&self) -> &'static str {
        "github_releases"
    }
    fn description(&self) -> &'static str {
        "List a GitHub repository's releases (newest first): tag, name, date, and release notes. \
        Accepts `owner/repo` or a github.com URL. Keyless (set [github].token to raise the API rate \
        limit). Use for changelogs or 'what changed in version X'."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GithubReleasesArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<GithubReleasesArgs>()?;
            let repo = github_owner_repo(&args.repo)
                .ok_or_else(|| invalid(format!("not a GitHub owner/repo: '{}'", args.repo)))?;
            let max = clamp(args.max_results, 5, 30);
            let prereleases = args.include_prereleases.unwrap_or(false);
            let key = format!("ghrel|{repo}|{max}|{prereleases}");
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let per = if prereleases { max } else { (max * 3).min(100) }.to_string();
            let v = github_api(
                &server.http,
                &format!("/repos/{repo}/releases"),
                &server.github_token,
                &[("per_page", per.as_str())],
            )
            .await
            .map_err(internal)?;

            let empty = Vec::new();
            let mut out = format!("Releases for {repo}:\n");
            let mut shown = 0usize;
            for r in v.as_array().unwrap_or(&empty) {
                let pre = r
                    .get("prerelease")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(false);
                let draft = r.get("draft").and_then(|x| x.as_bool()).unwrap_or(false);
                if !prereleases && (pre || draft) {
                    continue;
                }
                let tag = r.get("tag_name").and_then(|x| x.as_str()).unwrap_or("");
                let name = r
                    .get("name")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(tag);
                let date = r
                    .get("published_at")
                    .and_then(|x| x.as_str())
                    .and_then(|d| d.get(..10))
                    .unwrap_or("");
                let url = r.get("html_url").and_then(|x| x.as_str()).unwrap_or("");
                let body = r.get("body").and_then(|x| x.as_str()).unwrap_or("").trim();
                shown += 1;
                out.push_str(&format!(
                    "\n{shown}. {name} ({tag}){} — {date}\n   {url}\n",
                    if pre { " [prerelease]" } else { "" }
                ));
                if !body.is_empty() {
                    out.push_str(&indent(&truncate_chars(body, 4000), "   "));
                    out.push('\n');
                }
                if shown >= max {
                    break;
                }
            }
            if shown == 0 {
                return Ok(text_result(format!(
                    "No {}releases found for {repo}.",
                    if prereleases { "" } else { "stable " }
                )));
            }
            let out = truncate_chars(&out, server.max_chars);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct GithubUser;
impl Skill for GithubUser {
    fn name(&self) -> &'static str {
        "github_user"
    }
    fn description(&self) -> &'static str {
        "Get a GitHub user's or org's public profile: name, bio, company, location, blog, public \
        repo count, followers. Accepts a username/login or github.com URL. Keyless."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GithubUserArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<GithubUserArgs>()?;
            let user = github_user_login(&args.user)
                .ok_or_else(|| invalid(format!("not a GitHub username: '{}'", args.user)))?;
            let key = format!("ghuser|{user}");
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let v = github_api(
                &server.http,
                &format!("/users/{user}"),
                &server.github_token,
                &[],
            )
            .await
            .map_err(internal)?;
            let out = format_user(&v, &user);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct GithubRepo;
impl Skill for GithubRepo {
    fn name(&self) -> &'static str {
        "github_repo"
    }
    fn description(&self) -> &'static str {
        "Get a GitHub repository's metadata: description, stars, forks, primary language, topics, \
        license, default branch, homepage, and timestamps. Accepts `owner/repo` or a github.com \
        URL. Keyless."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<GithubRepoArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<GithubRepoArgs>()?;
            let repo = github_owner_repo(&args.repo)
                .ok_or_else(|| invalid(format!("not a GitHub owner/repo: '{}'", args.repo)))?;
            let key = format!("ghrepo|{repo}");
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }
            let v = github_api(
                &server.http,
                &format!("/repos/{repo}"),
                &server.github_token,
                &[],
            )
            .await
            .map_err(internal)?;
            let out = format_repo(&v, &repo);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(GithubReleases),
        Box::new(GithubUser),
        Box::new(GithubRepo),
    ]
}

#[cfg(test)]
mod tests {
    use super::{github_owner_repo, github_user_login};

    #[test]
    fn owner_repo_from_shorthand_and_urls() {
        assert_eq!(
            github_owner_repo("rust-lang/rust").as_deref(),
            Some("rust-lang/rust")
        );
        assert_eq!(
            github_owner_repo("https://github.com/rust-lang/rust").as_deref(),
            Some("rust-lang/rust")
        );
        assert_eq!(
            github_owner_repo("https://github.com/rust-lang/rust/releases").as_deref(),
            Some("rust-lang/rust")
        );
        assert_eq!(
            github_owner_repo("github.com/a/b.git").as_deref(),
            Some("a/b")
        );
        assert_eq!(github_owner_repo("not-a-repo"), None);
    }

    #[test]
    fn user_login_from_shorthand_and_urls() {
        assert_eq!(github_user_login("rust-lang").as_deref(), Some("rust-lang"));
        assert_eq!(github_user_login("@octocat").as_deref(), Some("octocat"));
        assert_eq!(
            github_user_login("https://github.com/torvalds").as_deref(),
            Some("torvalds")
        );
    }
}
