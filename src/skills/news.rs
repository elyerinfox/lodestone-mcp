//! News-feed skill (keyless): fetch a syndication feed and return its recent
//! items. Generalizes the Medium tag-RSS provider to any **RSS 2.0 or Atom** feed,
//! parsed with `roxmltree` so both formats (and CDATA/HTML bodies) are handled.
//!
//! `source` is either a full feed URL or one of a few built-in shorthands. Always
//! on (read-only, public data) and cached; no API key.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::util::{decode_entities, html_to_text, truncate_chars};
use crate::{internal, invalid, text_result};

/// A few stable, keyless feeds so the model can say "hackernews" instead of a URL.
fn known_source(s: &str) -> Option<&'static str> {
    Some(match s.trim().to_ascii_lowercase().as_str() {
        "hackernews" | "hn" => "https://hnrss.org/frontpage",
        "bbc" => "https://feeds.bbci.co.uk/news/rss.xml",
        "theverge" | "verge" => "https://www.theverge.com/rss/index.xml",
        "arstechnica" | "ars" => "https://feeds.arstechnica.com/arstechnica/index",
        "lobsters" => "https://lobste.rs/rss",
        "lwn" => "https://lwn.net/headlines/rss",
        _ => return None,
    })
}

/// Resolve a `source` argument to a feed URL: a built-in shorthand, or an
/// http(s) URL passed through verbatim.
fn resolve_source(source: &str) -> Result<String, McpError> {
    let s = source.trim();
    if let Some(url) = known_source(s) {
        return Ok(url.to_string());
    }
    if s.starts_with("http://") || s.starts_with("https://") {
        return Ok(s.to_string());
    }
    Err(invalid(format!(
        "'{s}' is not a known source or feed URL — pass an RSS/Atom URL (https://…) or one of: \
         hackernews, bbc, theverge, arstechnica, lobsters, lwn"
    )))
}

struct FeedItem {
    title: String,
    link: String,
    date: String,
    summary: String,
}

/// First direct-child element text (CDATA included) whose local name matches, if any.
fn child_text(node: roxmltree::Node, name: &str) -> Option<String> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case(name))
        .and_then(|c| c.text())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Pick an Atom entry's display link: prefer `rel="alternate"` (or no rel), else
/// the first `<link href>`.
fn atom_link(node: roxmltree::Node) -> Option<String> {
    let links: Vec<_> = node
        .children()
        .filter(|c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case("link"))
        .collect();
    let pick = links
        .iter()
        .find(|l| l.attribute("rel") == Some("alternate"))
        .or_else(|| links.iter().find(|l| l.attribute("rel").is_none()))
        .or_else(|| links.first());
    pick.and_then(|l| l.attribute("href")).map(str::to_string)
}

/// Parse an RSS-2.0 (`<item>`) or Atom (`<entry>`) feed into items (up to `max`),
/// returning `(feed_title, items)`.
fn parse_feed(xml: &str, max: usize) -> Result<(String, Vec<FeedItem>)> {
    let doc = roxmltree::Document::parse(xml)?;
    let root = doc.root_element();

    // Feed-level title: the channel/feed's own <title> (an item also has one, so
    // take a <title> whose parent is channel/feed, not item/entry).
    let feed_title = doc
        .descendants()
        .find(|n| {
            n.is_element()
                && n.tag_name().name().eq_ignore_ascii_case("title")
                && n.parent().is_some_and(|p| {
                    let pn = p.tag_name().name();
                    pn.eq_ignore_ascii_case("channel") || pn.eq_ignore_ascii_case("feed")
                })
        })
        .and_then(|n| n.text())
        .map(|t| decode_entities(t.trim()))
        .unwrap_or_default();
    let _ = root;

    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| {
        n.is_element() && {
            let t = n.tag_name().name();
            t.eq_ignore_ascii_case("item") || t.eq_ignore_ascii_case("entry")
        }
    }) {
        let is_atom = node.tag_name().name().eq_ignore_ascii_case("entry");
        let link = if is_atom {
            atom_link(node).unwrap_or_default()
        } else {
            child_text(node, "link").unwrap_or_default()
        };
        let title = child_text(node, "title")
            .map(|t| decode_entities(&t))
            .unwrap_or_else(|| "(untitled)".to_string());
        let date = child_text(node, "pubDate")
            .or_else(|| child_text(node, "published"))
            .or_else(|| child_text(node, "updated"))
            .unwrap_or_default();
        let summary_raw = child_text(node, "description")
            .or_else(|| child_text(node, "summary"))
            .or_else(|| child_text(node, "content"))
            .unwrap_or_default();
        let summary = truncate_chars(&html_to_text(&summary_raw), 240);
        if link.is_empty() && title == "(untitled)" {
            continue;
        }
        out.push(FeedItem {
            title,
            link,
            date,
            summary,
        });
        if out.len() >= max {
            break;
        }
    }
    Ok((feed_title, out))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NewsArgs {
    /// A feed URL (RSS 2.0 or Atom, `https://…`) or a built-in shorthand:
    /// `hackernews`, `bbc`, `theverge`, `arstechnica`, `lobsters`, `lwn`.
    source: String,
    /// Max items to return (default 15, capped at 50).
    #[serde(default)]
    max_results: Option<usize>,
}

pub struct NewsFeed;
impl Skill for NewsFeed {
    fn name(&self) -> &'static str {
        "news_feed"
    }
    fn description(&self) -> &'static str {
        "Fetch recent items from an RSS/Atom news feed (keyless). `source` is a feed URL \
        (https://…) or a built-in shorthand (hackernews, bbc, theverge, arstechnica, lobsters, \
        lwn). Returns each item's title, link, date, and a short summary. Read-only; cached."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NewsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<NewsArgs>()?;
            let url = resolve_source(&args.source)?;
            let max = args.max_results.unwrap_or(15).clamp(1, 50);
            let key = format!("news_feed|{url}|{max}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let xml = server
                .http
                .get(&url)
                .header(
                    "Accept",
                    "application/rss+xml, application/atom+xml, application/xml, text/xml",
                )
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
                .map_err(|e| internal(e.into()))?
                .text()
                .await
                .map_err(|e| internal(e.into()))?;
            let (feed_title, items) = parse_feed(&xml, max).map_err(internal)?;
            if items.is_empty() {
                return Err(invalid(format!(
                    "no items parsed from '{url}' (not an RSS/Atom feed?)"
                )));
            }
            let header = if feed_title.is_empty() {
                format!("{} item(s) from {url}", items.len())
            } else {
                format!("{feed_title} — {} item(s)", items.len())
            };
            let mut lines = vec![header];
            for (i, it) in items.iter().enumerate() {
                lines.push(format!("\n{}. {}", i + 1, it.title));
                if !it.link.is_empty() {
                    lines.push(format!("   {}", it.link));
                }
                let mut meta = String::new();
                if !it.date.is_empty() {
                    meta.push_str(&it.date);
                }
                if !it.summary.is_empty() {
                    if !meta.is_empty() {
                        meta.push_str(" — ");
                    }
                    meta.push_str(&it.summary);
                }
                if !meta.is_empty() {
                    lines.push(format!("   {meta}"));
                }
            }
            let report = lines.join("\n");
            server.retrieval_put(key, &report);
            Ok(text_result(report))
        })
    }
}

/// Always-on (read-only); still gateable via `[tools]`.
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(NewsFeed)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_shorthands_and_urls() {
        assert_eq!(
            resolve_source("hackernews").unwrap(),
            "https://hnrss.org/frontpage"
        );
        assert_eq!(
            resolve_source("https://example.com/feed.xml").unwrap(),
            "https://example.com/feed.xml"
        );
        assert!(resolve_source("not a feed").is_err());
    }

    #[test]
    fn parses_rss_2() {
        let xml = r#"<?xml version="1.0"?>
        <rss version="2.0"><channel>
          <title>Example News</title>
          <item>
            <title><![CDATA[First Post]]></title>
            <link>https://ex.com/1</link>
            <pubDate>Mon, 01 Jan 2026 12:00:00 GMT</pubDate>
            <description><![CDATA[<p>Some <b>body</b> text.</p>]]></description>
          </item>
          <item>
            <title>Second Post</title>
            <link>https://ex.com/2</link>
          </item>
        </channel></rss>"#;
        let (title, items) = parse_feed(xml, 10).unwrap();
        assert_eq!(title, "Example News");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "First Post");
        assert_eq!(items[0].link, "https://ex.com/1");
        assert!(items[0].summary.contains("body"));
        assert_eq!(items[1].title, "Second Post");
    }

    #[test]
    fn parses_atom() {
        let xml = r#"<?xml version="1.0"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
          <title>Atom Example</title>
          <entry>
            <title>Atom Entry</title>
            <link rel="alternate" href="https://ex.com/atom/1"/>
            <updated>2026-01-01T12:00:00Z</updated>
            <summary>A short summary.</summary>
          </entry>
        </feed>"#;
        let (title, items) = parse_feed(xml, 10).unwrap();
        assert_eq!(title, "Atom Example");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "Atom Entry");
        assert_eq!(items[0].link, "https://ex.com/atom/1");
        assert!(items[0].summary.contains("short summary"));
    }

    #[test]
    fn caps_results() {
        let mut xml = String::from(r#"<rss><channel><title>T</title>"#);
        for i in 0..30 {
            xml.push_str(&format!(
                "<item><title>P{i}</title><link>https://ex.com/{i}</link></item>"
            ));
        }
        xml.push_str("</channel></rss>");
        let (_t, items) = parse_feed(&xml, 5).unwrap();
        assert_eq!(items.len(), 5);
    }
}
