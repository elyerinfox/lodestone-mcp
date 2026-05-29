//! DuckDuckGo engine spec. Two interchangeable HTML endpoints are declared so the
//! provider can **rotate** between them and **fall back** under load (DuckDuckGo
//! rate-limits aggressively by IP):
//!
//! * primary — `lite.duckduckgo.com/lite/` (POST form, table layout, direct links);
//! * alt — `html.duckduckgo.com/html/` (GET, `result__*` markup, links wrapped in a
//!   `/l/?uddg=…` redirect that this module decodes).
//!
//! Both honor `site:`, so code search scopes precisely. It's still typically paired
//! with a more tolerant fallback engine (Mojeek) in the provider chain.

use scraper::{Html, Selector};

use super::{CodeScope, Endpoint, EngineSpec, Extract, Method};
use crate::provider::SearchResult;
use crate::util::collapse_ws;

pub(super) static SPEC: EngineSpec = EngineSpec {
    id: "duckduckgo",
    url: "https://lite.duckduckgo.com/lite/",
    method: Method::PostForm,
    extract: Extract::Selectors {
        link: "a.result-link",
        snippet: "td.result-snippet",
    },
    alts: &[Endpoint {
        url: "https://html.duckduckgo.com/html/",
        method: Method::Get,
        extract: Extract::Custom(parse_html),
    }],
    code_scope: CodeScope::SiteOperator,
    extra_params: &[],
};

/// Parser for the `html.duckduckgo.com/html/` layout: `a.result__a` titles +
/// `.result__snippet` bodies, with the redirect-wrapped href decoded to its target.
fn parse_html(body: &str, max: usize) -> Vec<SearchResult> {
    let doc = Html::parse_document(body);
    let link_sel = Selector::parse("a.result__a").unwrap();
    let snip_sel = Selector::parse("a.result__snippet, .result__snippet").unwrap();
    let links: Vec<_> = doc.select(&link_sel).collect();
    let snips: Vec<_> = doc.select(&snip_sel).collect();

    let mut out = Vec::new();
    for (i, a) in links.iter().enumerate() {
        let href = a.value().attr("href").unwrap_or("").trim();
        let url = decode_redirect(href);
        if url.is_empty() {
            continue;
        }
        let title = collapse_ws(&a.text().collect::<String>());
        let snippet = snips
            .get(i)
            .map(|s| collapse_ws(&s.text().collect::<String>()))
            .unwrap_or_default();
        out.push(SearchResult {
            title,
            url,
            snippet,
            ..Default::default()
        });
        if out.len() >= max {
            break;
        }
    }
    out
}

/// DuckDuckGo's HTML results wrap the real URL in a `//duckduckgo.com/l/?uddg=<enc>`
/// redirect; pull out and percent-decode the `uddg` target. Non-redirect hrefs are
/// returned as-is (with a scheme added for protocol-relative `//host/…` links).
fn decode_redirect(href: &str) -> String {
    if href.is_empty() {
        return String::new();
    }
    let abs = match href.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => href.to_string(),
    };
    if abs.contains("uddg=") {
        if let Ok(u) = url::Url::parse(&abs) {
            if let Some((_, v)) = u.query_pairs().find(|(k, _)| k == "uddg") {
                return v.into_owned();
            }
        }
    }
    abs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_endpoint_decodes_redirect_links() {
        let html = r##"<html><body>
            <div class="result results_links">
              <h2 class="result__title">
                <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fpage&amp;rut=abc">Example Title</a>
              </h2>
              <a class="result__snippet" href="//duckduckgo.com/l/?uddg=x">A snippet here</a>
            </div>
        </body></html>"##;
        let out = parse_html(html, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://example.com/page");
        assert_eq!(out[0].title, "Example Title");
        assert_eq!(out[0].snippet, "A snippet here");
    }

    #[test]
    fn decode_passthrough_and_protocol_relative() {
        assert_eq!(decode_redirect("https://a.test/x"), "https://a.test/x");
        assert_eq!(decode_redirect("//a.test/x"), "https://a.test/x");
        assert_eq!(decode_redirect(""), "");
    }
}
