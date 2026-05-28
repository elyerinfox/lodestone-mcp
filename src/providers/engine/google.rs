//! Google engine spec (feature `google`). Scrapes google.com via the shared
//! headless browser (`Method::Browser`) so the request looks like a real
//! browser. Google is CAPTCHA-prone on datacenter IPs and has a regional consent
//! page, so keep a tolerant engine (Mojeek) in the chain as a fallback. Its
//! markup needs real logic, so it uses a custom `Extract` rather than selectors.

use scraper::{Html, Selector};

use super::{CodeScope, EngineSpec, Extract, Method};
use crate::provider::SearchResult;
use crate::util::collapse_ws;

pub(super) static SPEC: EngineSpec = EngineSpec {
    id: "google",
    url: "https://www.google.com/search",
    method: Method::Browser,
    extract: Extract::Custom(parse),
    code_scope: CodeScope::SiteOperator,
    extra_params: &[("hl", "en"), ("gl", "us"), ("num", "20")],
};

fn parse(html: &str, max: usize) -> Vec<SearchResult> {
    let doc = Html::parse_document(html);
    let h3_sel = Selector::parse("h3").unwrap();
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for h3 in doc.select(&h3_sel) {
        // Walk up to the enclosing result anchor.
        let mut href = None;
        for ancestor in h3.ancestors() {
            if let Some(el) = ancestor.value().as_element() {
                if el.name() == "a" {
                    if let Some(h) = el.attr("href") {
                        href = Some(h.to_string());
                        break;
                    }
                }
            }
        }
        let url = match href.as_deref().and_then(clean_href) {
            Some(u) => u,
            None => continue,
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let title = collapse_ws(&h3.text().collect::<String>());
        if title.is_empty() {
            continue;
        }
        out.push(SearchResult {
            title,
            url,
            ..Default::default()
        });
        if out.len() >= max {
            break;
        }
    }

    if out.is_empty() && looks_blocked(html) {
        tracing::warn!("google returned a CAPTCHA / consent page instead of results");
    }
    out
}

/// Heuristic: does this look like Google's anti-bot / consent interstitial
/// rather than a results page?
fn looks_blocked(html: &str) -> bool {
    let h = html.to_ascii_lowercase();
    h.contains("/sorry/")
        || h.contains("unusual traffic")
        || h.contains("recaptcha")
        || h.contains("our systems have detected")
        || h.contains("before you continue to google")
}

/// Normalize a Google result href to a real destination URL, dropping internal
/// links. Handles both direct hrefs and the legacy `/url?q=` redirector.
fn clean_href(href: &str) -> Option<String> {
    if href.starts_with("http") {
        if href.contains("google.com") {
            return None;
        }
        return Some(href.to_string());
    }
    if let Some(rest) = href.strip_prefix("/url?") {
        for (k, v) in url::form_urlencoded::parse(rest.as_bytes()) {
            if k == "q" {
                return Some(v.into_owned());
            }
        }
    }
    None
}
