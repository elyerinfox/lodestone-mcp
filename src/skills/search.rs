//! Search skills — the general `web_search` / `code_search` / `docs_search` /
//! `qa_search` tools (which run the provider [`Registry`]), the StackOverflow
//! answer reader, and the auto-generated per-provider `<kind>_<id>` tools. Also
//! owns the search result formatters.

use std::sync::{Arc, LazyLock};

use anyhow::Result;
use futures::future::BoxFuture;
use regex::Regex;
use reqwest::Client;
use rmcp::handler::server::router::tool::ToolRoute;
use rmcp::handler::server::tool::{parse_json_object, schema_for_type, ToolCallContext};
use rmcp::model::{CallToolResult, JsonObject, Tool};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::provider::{ProviderKind, Registry, SearchQuery, SearchResult};
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::indent;
use crate::{clamp, internal, invalid, text_result, util, Lodestone};

// ---------------------------------------------------------------------------
// StackExchange thread retrieval (keyless public API) — backs qa_stackoverflow_answers
// ---------------------------------------------------------------------------

static QUESTION_ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/questions/(\d+)").unwrap());

/// Extract a numeric question id from a bare id or a StackExchange question URL.
fn extract_question_id(input: &str) -> Option<String> {
    let t = input.trim();
    if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()) {
        return Some(t.to_string());
    }
    QUESTION_ID_RE.captures(t).map(|c| c[1].to_string())
}

/// Fetch `(question_json, answers_json)` for a question id. An optional API key
/// (empty = none) raises the per-IP quota.
async fn se_answers(
    client: &Client,
    question_id: &str,
    site: &str,
    max: usize,
    key: &str,
) -> Result<(Value, Value)> {
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

/// Render a StackOverflow question page and format its question + top answers as
/// text, mirroring the API path's output. Returns the page URL used.
async fn scrape_answers(url: &str, max: usize) -> Result<String> {
    use crate::browser::PageRenderer;
    let html = crate::browser::shared_global().render(url).await?;
    if html.to_ascii_lowercase().contains("captcha") {
        return Err(anyhow::anyhow!("StackOverflow served a CAPTCHA page"));
    }
    parse_answers_page(&html, url, max)
        .ok_or_else(|| anyhow::anyhow!("could not parse the question page"))
}

/// Parse a StackOverflow question page into the same text shape as the API path.
fn parse_answers_page(html: &str, url: &str, max: usize) -> Option<String> {
    use scraper::{CaseSensitivity, Html, Selector};

    let doc = Html::parse_document(html);
    let title_sel = Selector::parse("#question-header h1, .question-hyperlink").unwrap();
    let q_body_sel = Selector::parse("#question .s-prose").unwrap();
    let answer_sel = Selector::parse(".answer").unwrap();
    let body_sel = Selector::parse(".s-prose").unwrap();

    let title = doc
        .select(&title_sel)
        .next()
        .map(|e| util::collapse_ws(&e.text().collect::<String>()))
        .unwrap_or_default();
    let q_body = doc
        .select(&q_body_sel)
        .next()
        .map(|e| util::html_to_text(&e.inner_html()))
        .unwrap_or_default();
    if title.is_empty() && q_body.is_empty() {
        return None;
    }

    let mut out = format!("QUESTION: {title}\n{url}\n\n{}\n\n", q_body.trim());

    let mut answers: Vec<(i64, bool, String)> = Vec::new();
    for ans in doc.select(&answer_sel) {
        let el = ans.value();
        let score = el
            .attr("data-score")
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0);
        let accepted = el.has_class("accepted-answer", CaseSensitivity::AsciiCaseInsensitive);
        let body = ans
            .select(&body_sel)
            .next()
            .map(|e| util::html_to_text(&e.inner_html()))
            .unwrap_or_default();
        answers.push((score, accepted, body));
        if answers.len() >= max {
            break;
        }
    }

    if answers.is_empty() {
        out.push_str("(no answers)");
    } else {
        out.push_str(&format!("===== {} ANSWER(S) =====\n", answers.len()));
        for (i, (score, accepted, body)) in answers.iter().enumerate() {
            out.push_str(&format!(
                "\n----- Answer {} (score {score}{}) -----\n",
                i + 1,
                if *accepted { ", accepted ✓" } else { "" }
            ));
            out.push_str(body.trim());
            out.push('\n');
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Argument schemas
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WebSearchArgs {
    /// The search query. Search-engine operators work (e.g. quotes, `site:`).
    query: String,
    /// Maximum number of results to return. Default 8, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
    /// Fetch results through a real headless browser (executes JS, can bypass
    /// bot-walls/rate-limits) instead of plain HTTP. Slower; needs a local
    /// Chrome/Chromium at runtime.
    #[serde(default)]
    render: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CodeSearchArgs {
    /// What to look for in source code (symbol, function name, snippet, etc.).
    query: String,
    /// Optional language hint to narrow results (e.g. "rust", "python").
    #[serde(default)]
    language: Option<String>,
    /// Maximum number of results to return. Default 10, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
    /// Fetch results through a real headless browser instead of plain HTTP.
    #[serde(default)]
    render: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DocsSearchArgs {
    /// What to look for — a library/package name, API, or documentation topic.
    query: String,
    /// Maximum number of results to return. Default 10, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
    /// Fetch the framework-doc site searches through a real headless browser
    /// instead of plain HTTP. Ignored by the JSON registry providers.
    #[serde(default)]
    render: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StackSearchArgs {
    /// The question/problem to search for.
    query: String,
    /// StackExchange site to search. Defaults to the configured site
    /// (e.g. "serverfault", "superuser", "askubuntu", "unix").
    #[serde(default)]
    site: Option<String>,
    /// Maximum number of results to return. Default 8, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
    /// Scrape stackoverflow.com via a headless browser instead of the API
    /// (avoids the API quota; stackoverflow site only). Needs a local
    /// Chrome/Chromium at runtime.
    #[serde(default)]
    render: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StackAnswersArgs {
    /// A StackExchange question URL or numeric question id.
    question: String,
    /// StackExchange site. Defaults to the configured site. Must match the question's site.
    #[serde(default)]
    site: Option<String>,
    /// Maximum number of answers to return (sorted by votes). Default 3, cap 10.
    #[serde(default)]
    max_answers: Option<u32>,
    /// Scrape the question page via the headless browser instead of the API
    /// (saves API quota). Only applies to the `stackoverflow` site; other sites
    /// fall back to the API. Needs a local Chrome/Chromium at runtime.
    #[serde(default)]
    render: Option<bool>,
}

/// Arguments for the granular, one-tool-per-provider skills.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ProviderSearchArgs {
    /// The search query.
    query: String,
    /// Maximum number of results to return. Default 10, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
    /// Optional language hint (code providers).
    #[serde(default)]
    language: Option<String>,
    /// Optional StackExchange site slug (qa providers).
    #[serde(default)]
    site: Option<String>,
    /// Fetch via a real headless browser instead of plain HTTP.
    #[serde(default)]
    render: Option<bool>,
}

// ---------------------------------------------------------------------------
// Formatting
// ---------------------------------------------------------------------------

/// Local current date (YYYY-MM-DD) stamped onto result headers so the model can
/// anchor recency instead of guessing — web snippets often omit the year.
fn now_stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn format_web(query: &str, engine: &str, hits: &[SearchResult]) -> String {
    let mut out = format!(
        "Web results for \"{query}\" (current date {}; via {engine}):\n",
        now_stamp()
    );
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n   {}\n", i + 1, h.title, h.url));
        if !h.snippet.is_empty() {
            out.push_str(&format!("   {}\n", h.snippet));
        }
        if let Some(meta) = &h.meta {
            out.push_str(&format!("   [{meta}]\n"));
        }
    }
    out
}

fn format_code(query: &str, engine: &str, hits: &[SearchResult]) -> String {
    let mut out = format!(
        "Code results for \"{query}\" (current date {}; via {engine}):\n",
        now_stamp()
    );
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n", i + 1, h.title));
        if !h.url.is_empty() {
            out.push_str(&format!("   {}\n", h.url));
        }
        if !h.snippet.is_empty() {
            out.push_str(&indent(&h.snippet, "   "));
            out.push('\n');
        }
        if let Some(meta) = &h.meta {
            out.push_str(&format!("   [{meta}]\n"));
        }
    }
    out
}

fn format_docs(query: &str, engine: &str, hits: &[SearchResult]) -> String {
    let mut out = format!(
        "Documentation results for \"{query}\" (current date {}; via {engine}):\n",
        now_stamp()
    );
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!("\n{}. {}\n   {}\n", i + 1, h.title, h.url));
        if !h.snippet.is_empty() {
            out.push_str(&format!("   {}\n", h.snippet));
        }
    }
    out
}

fn format_qa(query: &str, site: &str, hits: &[SearchResult]) -> String {
    let mut out = format!(
        "{site} results for \"{query}\" (current date {}):\n",
        now_stamp()
    );
    for (i, h) in hits.iter().enumerate() {
        let score = h.score.unwrap_or(0);
        out.push_str(&format!("\n{}. {}\n", i + 1, h.title));
        out.push_str(&format!("   score {score}"));
        if let Some(meta) = &h.meta {
            out.push_str(&format!(" · {meta}"));
        }
        out.push('\n');
        if !h.url.is_empty() {
            out.push_str(&format!("   {}\n", h.url));
        }
    }
    out.push_str("\nTip: pass a question URL to qa_stackoverflow_answers to read answers.");
    out
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

pub struct WebSearch;
impl Skill for WebSearch {
    fn name(&self) -> &'static str {
        "web_search"
    }
    fn description(&self) -> &'static str {
        "Search the web (scraped via the configured web providers, no API key). Returns a ranked \
        list of title / URL / snippet. Use `fetch_page` to read a result. Set render=true to fetch \
        via a real headless browser (slower, but can bypass rate-limits/bot-walls)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WebSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<WebSearchArgs>()?;
            let q = SearchQuery {
                text: args.query.clone(),
                language: None,
                site: None,
                limit: clamp(args.max_results, 8, 25),
                render: args.render.unwrap_or(false),
            };
            let (hits, engine) = server
                .registry
                .search(ProviderKind::Web, &server.http, &q)
                .await;
            if hits.is_empty() {
                return Ok(text_result(format!("No web results for: {}", args.query)));
            }
            Ok(text_result(format_web(&args.query, &engine, &hits)))
        })
    }
}

pub struct CodeSearch;
impl Skill for CodeSearch {
    fn name(&self) -> &'static str {
        "code_search"
    }
    fn description(&self) -> &'static str {
        "Search source code across public repositories (via the configured code providers, e.g. \
        grep.app then a GitHub-scoped web search). Returns repo, file path and a snippet. Use \
        `fetch_repo_file` on a result URL to read the full file. Set render=true to fetch via a real \
        headless browser (slower, but can bypass rate-limits/bot-walls)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CodeSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<CodeSearchArgs>()?;
            let q = SearchQuery {
                text: args.query.clone(),
                language: args.language.clone(),
                site: None,
                limit: clamp(args.max_results, 10, 25),
                render: args.render.unwrap_or(false),
            };
            let (hits, engine) = server
                .registry
                .search(ProviderKind::Code, &server.http, &q)
                .await;
            if hits.is_empty() {
                return Ok(text_result(format!("No code results for: {}", args.query)));
            }
            Ok(text_result(format_code(&args.query, &engine, &hits)))
        })
    }
}

pub struct DocsSearch;
impl Skill for DocsSearch {
    fn name(&self) -> &'static str {
        "docs_search"
    }
    fn description(&self) -> &'static str {
        "Search developer documentation and package registries (crates.io, npm, MDN, …) and \
        framework/tooling docs (PHP, Laravel, Vue, React, Svelte, Docker, Kubernetes, …) — no API \
        key. Returns matching packages/pages with name, version, URL and description. Then \
        `fetch_page` to read a result."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DocsSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DocsSearchArgs>()?;
            let q = SearchQuery {
                text: args.query.clone(),
                language: None,
                site: None,
                limit: clamp(args.max_results, 10, 25),
                render: args.render.unwrap_or(false),
            };
            let (hits, engine) = server
                .registry
                .search(ProviderKind::Docs, &server.http, &q)
                .await;
            if hits.is_empty() {
                return Ok(text_result(format!(
                    "No documentation results for: {}",
                    args.query
                )));
            }
            Ok(text_result(format_docs(&args.query, &engine, &hits)))
        })
    }
}

pub struct QaSearch;
impl Skill for QaSearch {
    fn name(&self) -> &'static str {
        "qa_search"
    }
    fn description(&self) -> &'static str {
        "Search the configured Q&A providers (currently the StackExchange network: StackOverflow, \
        Server Fault, Super User, Ask Ubuntu, …). Returns matching questions with score, answer \
        count and links. Uses the keyless API by default; set render=true to scrape via a headless \
        browser (no API quota). To search a single site directly use the per-provider tool \
        qa_stackoverflow; use qa_stackoverflow_answers to read the actual answers."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StackSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<StackSearchArgs>()?;
            let site = args
                .site
                .clone()
                .unwrap_or_else(|| server.default_se_site.to_string());
            if !server.se_site_allowed(&site) {
                return Err(invalid(format!(
                    "site '{site}' is not in the configured StackExchange allowlist"
                )));
            }
            let q = SearchQuery {
                text: args.query.clone(),
                language: None,
                site: Some(site.clone()),
                limit: clamp(args.max_results, 8, 25),
                render: args.render.unwrap_or(false),
            };
            let (hits, _engine) = server
                .registry
                .search(ProviderKind::Qa, &server.http, &q)
                .await;
            if hits.is_empty() {
                return Ok(text_result(format!(
                    "No {site} results for: {}",
                    args.query
                )));
            }
            Ok(text_result(format_qa(&args.query, &site, &hits)))
        })
    }
}

pub struct QaStackoverflowAnswers;
impl Skill for QaStackoverflowAnswers {
    fn name(&self) -> &'static str {
        "qa_stackoverflow_answers"
    }
    fn description(&self) -> &'static str {
        "Read a StackOverflow/StackExchange question body and its top answers (by votes), including \
        any code blocks. Accepts a question URL or numeric id. Uses the keyless API by default; set \
        render=true to scrape the stackoverflow.com page instead (saves API quota). Provider-specific \
        to the StackExchange network."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StackAnswersArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::SearchEngine,
        }
    }

    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<StackAnswersArgs>()?;
            let site = args
                .site
                .clone()
                .unwrap_or_else(|| server.default_se_site.to_string());
            if !server.se_site_allowed(&site) {
                return Err(invalid(format!(
                    "site '{site}' is not in the configured StackExchange allowlist"
                )));
            }
            let max = clamp(args.max_answers, 3, 10);
            let qid = extract_question_id(&args.question).ok_or_else(|| {
                invalid(format!(
                    "could not find a question id in '{}'",
                    args.question
                ))
            })?;

            // Render scraping (no API quota) applies only to stackoverflow.com;
            // other sites always use the API.
            let render = args.render.unwrap_or(false) && site == "stackoverflow";
            let key = format!("se_answers|{site}|{max}|{qid}|{render}");
            if let Some(cached) = server.retrieval_get(&key).await {
                return Ok(text_result(cached));
            }

            if render {
                let page_url = if args.question.trim().starts_with("http") {
                    args.question.trim().to_string()
                } else {
                    format!("https://stackoverflow.com/questions/{qid}")
                };
                let out = scrape_answers(&page_url, max).await.map_err(internal)?;
                let out = util::truncate_chars(&out, server.max_chars);
                server.retrieval_put(key, &out);
                return Ok(text_result(out));
            }

            let (q, a) = se_answers(&server.http, &qid, &site, max, &server.se_key)
                .await
                .map_err(internal)?;

            let mut out = String::new();
            if let Some(item) = q
                .get("items")
                .and_then(|i| i.as_array())
                .and_then(|a| a.first())
            {
                let title =
                    util::decode_entities(item.get("title").and_then(|x| x.as_str()).unwrap_or(""));
                let link = item.get("link").and_then(|x| x.as_str()).unwrap_or("");
                let body = item.get("body").and_then(|x| x.as_str()).unwrap_or("");
                out.push_str(&format!("QUESTION: {title}\n{link}\n\n"));
                out.push_str(&util::html_to_text(body));
                out.push_str("\n\n");
            } else {
                return Ok(text_result(format!("Question {qid} not found on {site}.")));
            }

            match a.get("items").and_then(|i| i.as_array()) {
                Some(list) if !list.is_empty() => {
                    out.push_str(&format!("===== {} ANSWER(S) =====\n", list.len()));
                    for (i, ans) in list.iter().enumerate() {
                        let score = ans.get("score").and_then(|x| x.as_i64()).unwrap_or(0);
                        let accepted = ans
                            .get("is_accepted")
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false);
                        let body = ans.get("body").and_then(|x| x.as_str()).unwrap_or("");
                        out.push_str(&format!(
                            "\n----- Answer {} (score {score}{}) -----\n",
                            i + 1,
                            if accepted { ", accepted ✓" } else { "" }
                        ));
                        out.push_str(&util::html_to_text(body));
                        out.push('\n');
                    }
                }
                _ => out.push_str("(no answers)"),
            }

            let out = util::truncate_chars(&out, server.max_chars);
            server.retrieval_put(key, &out);
            Ok(text_result(out))
        })
    }
}

/// The fixed search skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(WebSearch),
        Box::new(CodeSearch),
        Box::new(DocsSearch),
        Box::new(QaSearch),
        Box::new(QaStackoverflowAnswers),
    ]
}

// ---------------------------------------------------------------------------
// Per-provider tools (one direct tool per configured provider)
// ---------------------------------------------------------------------------

/// One direct tool per configured provider, named `<kind>_<id>` (e.g.
/// `web_mojeek`, `code_github`, `qa_stackoverflow`). These bypass the chain and
/// strategy, letting the model target a single source.
pub fn provider_routes(registry: &Registry) -> Vec<ToolRoute<Lodestone>> {
    let schema = schema_for_type::<ProviderSearchArgs>();
    registry
        .list()
        .into_iter()
        .map(|(kind, id)| {
            let name = format!("{}_{}", kind.as_str(), id);
            let description = format!(
                "Search the `{id}` {} provider directly (bypasses the configured chain and \
                 strategy). Use the general {}_search tool to query all configured {} providers.",
                kind.as_str(),
                kind.as_str(),
                kind.as_str(),
            );
            let tool = Tool::new(name, description, schema.clone());
            ToolRoute::new_dyn(tool, move |ctx| provider_call(ctx, kind, id))
        })
        .collect()
}

/// Handler shared by every per-provider tool: parse args, run that one provider,
/// format like its kind.
fn provider_call<'a>(
    ctx: ToolCallContext<'a, Lodestone>,
    kind: ProviderKind,
    id: &'static str,
) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
    Box::pin(async move {
        let svc = ctx.service;
        let args: ProviderSearchArgs = parse_json_object(ctx.arguments.unwrap_or_default())?;
        let q = SearchQuery {
            text: args.query,
            language: args.language,
            site: args.site,
            limit: clamp(args.max_results, 10, 25),
            render: args.render.unwrap_or(false),
        };
        let hits = svc.registry.run_one(kind, id, &svc.http, &q).await;
        let text = match kind {
            ProviderKind::Web => format_web(&q.text, id, &hits),
            ProviderKind::Code => format_code(&q.text, id, &hits),
            ProviderKind::Docs => format_docs(&q.text, id, &hits),
            ProviderKind::Qa => {
                let site = q.site.as_deref().unwrap_or("stackoverflow");
                format_qa(&q.text, site, &hits)
            }
        };
        Ok(text_result(text))
    })
}

#[cfg(test)]
mod tests {
    use super::{extract_question_id, parse_answers_page};

    #[test]
    fn question_id_from_url_or_bare() {
        assert_eq!(extract_question_id("12345").as_deref(), Some("12345"));
        assert_eq!(
            extract_question_id("https://stackoverflow.com/questions/231767/what-does-yield-do")
                .as_deref(),
            Some("231767")
        );
        assert_eq!(extract_question_id("not a question"), None);
    }

    #[test]
    fn scrape_parser_pulls_question_and_answers() {
        // Minimal shape of a StackOverflow question page.
        let html = r##"<html><body>
            <div id="question-header"><h1><a class="question-hyperlink">How to X?</a></h1></div>
            <div id="question" class="question" data-score="7">
              <div class="post-layout"><div class="s-prose"><p>The question body.</p></div></div>
            </div>
            <div id="answers">
              <div class="answer accepted-answer" data-score="42">
                <div class="s-prose"><p>The accepted answer.</p><pre><code>let x = 1;</code></pre></div>
              </div>
              <div class="answer" data-score="5">
                <div class="s-prose"><p>Another answer.</p></div>
              </div>
            </div>
        </body></html>"##;
        let out = parse_answers_page(html, "https://stackoverflow.com/questions/1", 10).unwrap();
        assert!(out.contains("QUESTION: How to X?"), "{out}");
        assert!(out.contains("The question body."), "{out}");
        assert!(out.contains("2 ANSWER(S)"), "{out}");
        assert!(out.contains("score 42, accepted ✓"), "{out}");
        assert!(out.contains("The accepted answer."), "{out}");
        assert!(out.contains("let x = 1;"), "{out}");
        assert!(out.contains("score 5)"), "{out}");
    }
}
