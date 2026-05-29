//! arXiv skills (keyless): `arxiv_search` searches the arXiv API by query and
//! `arxiv_get` fetches one paper's metadata by id. arXiv papers are open access —
//! each result includes the free PDF URL, so `read_pdf` can retrieve full text.
//!
//! The arXiv API returns Atom XML, parsed here with `roxmltree`.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use reqwest::Client;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::{collapse_ws, truncate_chars};
use crate::{clamp, internal, invalid, text_result};

const API: &str = "https://export.arxiv.org/api/query";

struct Entry {
    id: String,
    title: String,
    authors: Vec<String>,
    published: String,
    summary: String,
    categories: Vec<String>,
    abs_url: String,
    pdf_url: String,
}

/// Parse the arXiv Atom feed into entries.
fn parse_feed(xml: &str) -> Result<Vec<Entry>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| anyhow!("arXiv XML parse error: {e}"))?;
    let mut out = Vec::new();
    for e in doc.descendants().filter(|n| n.tag_name().name() == "entry") {
        let child = |name: &str| {
            e.children()
                .find(|c| c.tag_name().name() == name)
                .and_then(|c| c.text())
                .unwrap_or("")
        };
        let abs_url = child("id").trim().to_string();
        // arXiv id is the bit after /abs/ (keeps the version, e.g. 2103.00020v1).
        let id = abs_url
            .rsplit_once("/abs/")
            .map(|(_, r)| r)
            .unwrap_or(&abs_url)
            .to_string();
        let authors: Vec<String> = e
            .children()
            .filter(|c| c.tag_name().name() == "author")
            .filter_map(|a| {
                a.children()
                    .find(|c| c.tag_name().name() == "name")
                    .and_then(|n| n.text())
                    .map(|s| s.trim().to_string())
            })
            .collect();
        let categories: Vec<String> = e
            .children()
            .filter(|c| c.tag_name().name() == "category")
            .filter_map(|c| c.attribute("term").map(str::to_string))
            .collect();
        out.push(Entry {
            pdf_url: format!("https://arxiv.org/pdf/{id}"),
            id,
            title: collapse_ws(child("title")),
            authors,
            published: child("published").get(..10).unwrap_or("").to_string(),
            summary: collapse_ws(child("summary")),
            categories,
            abs_url,
        });
    }
    Ok(out)
}

/// Normalize an arXiv id from `2103.00020`, `arXiv:2103.00020v2`, an abs/pdf URL,
/// or an old-style `math/0211159`.
fn arxiv_id(input: &str) -> String {
    let mut s = input.trim();
    if let Some((_, r)) = s.rsplit_once("/abs/") {
        s = r;
    } else if let Some((_, r)) = s.rsplit_once("/pdf/") {
        s = r;
    }
    let s = s.strip_suffix(".pdf").unwrap_or(s).trim();
    let s = if s.len() >= 6 && s[..6].eq_ignore_ascii_case("arxiv:") {
        &s[6..]
    } else {
        s
    };
    s.trim().to_string()
}

async fn fetch(http: &Client, params: &[(&str, &str)]) -> Result<Vec<Entry>> {
    let xml = http
        .get(API)
        .query(params)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_feed(&xml)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ArxivSearchArgs {
    /// What to search for (title/abstract/author keywords).
    query: String,
    /// Maximum number of results. Default 8, capped at 25.
    #[serde(default)]
    max_results: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ArxivGetArgs {
    /// An arXiv id, e.g. `2103.00020`, `arXiv:2103.00020v2`, or an abs/pdf URL.
    id: String,
}

pub struct ArxivSearch;
impl Skill for ArxivSearch {
    fn name(&self) -> &'static str {
        "arxiv_search"
    }
    fn description(&self) -> &'static str {
        "Search arXiv (keyless) for papers by query. Returns title, authors, date, categories, a \
        short abstract, and the abs + free PDF URLs (use read_pdf on the PDF for full text). Use \
        arxiv_get for one paper's full abstract."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ArxivSearchArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ArxivSearchArgs>()?;
            let limit = clamp(args.max_results, 8, 25);
            let cache_key = format!("arxiv_search|{limit}|{}", args.query.trim());
            if let Some(cached) = server.retrieval_get(&cache_key).await {
                return Ok(text_result(cached));
            }
            let max = limit.to_string();
            let search = format!("all:{}", args.query);
            let entries = fetch(
                &server.http,
                &[
                    ("search_query", search.as_str()),
                    ("max_results", max.as_str()),
                    ("sortBy", "relevance"),
                ],
            )
            .await
            .map_err(internal)?;
            if entries.is_empty() {
                return Ok(text_result(format!(
                    "No arXiv papers match: {}",
                    args.query
                )));
            }
            let mut out = format!("arXiv results for \"{}\":\n", args.query);
            for e in entries.iter().take(limit) {
                out.push_str(&format!("\n{} ({})\n   {}", e.title, e.id, e.abs_url));
                if !e.authors.is_empty() {
                    let who: Vec<&str> = e.authors.iter().take(4).map(|s| s.as_str()).collect();
                    let more = if e.authors.len() > 4 { ", …" } else { "" };
                    out.push_str(&format!("\n   {}{more}", who.join(", ")));
                }
                if !e.published.is_empty() {
                    out.push_str(&format!("\n   {}", e.published));
                }
                if !e.categories.is_empty() {
                    out.push_str(&format!(" · {}", e.categories.join(", ")));
                }
                out.push_str(&format!(
                    "\n   PDF: {}\n   {}\n",
                    e.pdf_url,
                    truncate_chars(&e.summary, 280)
                ));
            }
            server.retrieval_put(cache_key, &out);
            Ok(text_result(out))
        })
    }
}

pub struct ArxivGet;
impl Skill for ArxivGet {
    fn name(&self) -> &'static str {
        "arxiv_get"
    }
    fn description(&self) -> &'static str {
        "Get an arXiv paper's metadata by id (keyless): title, authors, date, categories, full \
        abstract, and the abs + free PDF URLs. Use read_pdf on the PDF URL for the full text."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ArxivGetArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ArxivGetArgs>()?;
            let id = arxiv_id(&args.id);
            if id.is_empty() {
                return Err(invalid(format!("not an arXiv id: '{}'", args.id)));
            }
            let cache_key = format!("arxiv_get|{id}");
            if let Some(cached) = server.retrieval_get(&cache_key).await {
                return Ok(text_result(cached));
            }
            let entries = fetch(
                &server.http,
                &[("id_list", id.as_str()), ("max_results", "1")],
            )
            .await
            .map_err(internal)?;
            let Some(e) = entries.into_iter().next().filter(|e| !e.title.is_empty()) else {
                return Ok(text_result(format!("arXiv paper {id} not found.")));
            };
            let mut out = format!("{}\n  arXiv:{}\n  {}\n", e.title, e.id, e.abs_url);
            if !e.authors.is_empty() {
                out.push_str(&format!("  authors: {}\n", e.authors.join(", ")));
            }
            if !e.published.is_empty() {
                out.push_str(&format!("  published: {}\n", e.published));
            }
            if !e.categories.is_empty() {
                out.push_str(&format!("  categories: {}\n", e.categories.join(", ")));
            }
            out.push_str(&format!(
                "  PDF (read_pdf for full text): {}\n\n{}",
                e.pdf_url, e.summary
            ));
            server.retrieval_put(cache_key, &out);
            Ok(text_result(out))
        })
    }
}

/// The skills this module contributes.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(ArxivSearch), Box::new(ArxivGet)]
}

#[cfg(test)]
mod tests {
    use super::{arxiv_id, parse_feed};

    #[test]
    fn normalizes_ids() {
        assert_eq!(arxiv_id("2103.00020"), "2103.00020");
        assert_eq!(arxiv_id("arXiv:2103.00020v2"), "2103.00020v2");
        assert_eq!(
            arxiv_id("https://arxiv.org/abs/2103.00020v1"),
            "2103.00020v1"
        );
        assert_eq!(
            arxiv_id("https://arxiv.org/pdf/2103.00020.pdf"),
            "2103.00020"
        );
        assert_eq!(arxiv_id("math/0211159"), "math/0211159");
    }

    #[test]
    fn parses_atom_entry() {
        let xml = r#"<?xml version="1.0"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
          <entry>
            <id>https://arxiv.org/abs/1706.03762v5</id>
            <title>Attention Is All You Need</title>
            <published>2017-06-12T00:00:00Z</published>
            <summary>The dominant sequence transduction models...</summary>
            <author><name>Ashish Vaswani</name></author>
            <author><name>Noam Shazeer</name></author>
            <category term="cs.CL"/>
          </entry>
        </feed>"#;
        let e = parse_feed(xml).unwrap();
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].id, "1706.03762v5");
        assert_eq!(e[0].title, "Attention Is All You Need");
        assert_eq!(e[0].authors, ["Ashish Vaswani", "Noam Shazeer"]);
        assert_eq!(e[0].published, "2017-06-12");
        assert_eq!(e[0].pdf_url, "https://arxiv.org/pdf/1706.03762v5");
        assert_eq!(e[0].categories, ["cs.CL"]);
    }
}
