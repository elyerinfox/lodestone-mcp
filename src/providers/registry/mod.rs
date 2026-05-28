//! Documentation & package-registry providers (`docs` kind).
//!
//! Spec-driven family like `engine`/`forge`: every registry is a keyless JSON
//! search API that differs only in DECLARATIVE specifics captured by
//! [`RegistrySpec`] — the endpoint, the query/size params, a JSON pointer to the
//! results array, and pointers/templates mapping each item to title/url/
//! description/version. Adding one is a small file (`cratesio.rs`, `npm.rs`,
//! `mdn.rs`) that declares a spec; the shared [`RegistryProvider`] turns it into a
//! working provider.
//!
//! Golden rules: keyless and plain-HTTP by default; `render` isn't meaningful for
//! a JSON API, so it's accepted and ignored (like `grep_app`/`searxng`). Each
//! provider is independently enable/disable-able via `[providers].docs` and its
//! per-provider `docs_<id>` tool.

mod cratesio;
mod hex;
mod mdn;
mod npm;
mod nuget;
mod packagist;
mod rubygems;

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::provider::{ProviderKind, SearchProvider, SearchQuery, SearchResult};
use crate::util::collapse_ws;

/// How to pull each result field out of one item of the results array. Pointers
/// are JSON Pointers relative to the item (RFC 6901, e.g. `/name`).
pub(super) struct ItemMap {
    pub name: &'static str,
    pub description: &'static str,
    /// A pointer to a ready-made URL in the item …
    pub url_field: Option<&'static str>,
    /// … or a template like `https://crates.io/crates/{name}` filled with `name`.
    pub url_template: Option<&'static str>,
    /// Prepended to `url_field` values that are site-relative (e.g. `/en-US/…`).
    pub url_base: &'static str,
    /// Optional pointer to a version string (shown in title + meta).
    pub version: Option<&'static str>,
}

/// Declarative description of a keyless JSON registry search API.
pub(super) struct RegistrySpec {
    pub id: &'static str,
    /// Search endpoint.
    pub url: &'static str,
    /// Query-text parameter name (e.g. `q`, `text`).
    pub query_key: &'static str,
    /// Optional result-count parameter set to the requested limit (e.g. `per_page`).
    pub size_key: Option<&'static str>,
    pub extra_params: &'static [(&'static str, &'static str)],
    /// JSON pointer to the results array (e.g. `/crates`, `/objects`).
    pub results_ptr: &'static str,
    pub item: ItemMap,
}

/// Construct a registry provider by id, if known.
pub(super) fn make(id: &str) -> Option<RegistryProvider> {
    let spec = match id {
        "cratesio" => &cratesio::SPEC,
        "npm" => &npm::SPEC,
        "mdn" => &mdn::SPEC,
        "rubygems" => &rubygems::SPEC,
        "packagist" => &packagist::SPEC,
        "nuget" => &nuget::SPEC,
        "hex" => &hex::SPEC,
        _ => return None,
    };
    Some(RegistryProvider { spec })
}

/// A registry exposed as a standalone `docs` provider, driven by a [`RegistrySpec`].
pub(super) struct RegistryProvider {
    spec: &'static RegistrySpec,
}

#[async_trait]
impl SearchProvider for RegistryProvider {
    fn id(&self) -> &'static str {
        self.spec.id
    }
    fn kind(&self) -> ProviderKind {
        ProviderKind::Docs
    }
    async fn search(&self, http: &Client, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        let spec = self.spec;
        let limit = query.limit.to_string();
        let mut params: Vec<(&str, &str)> = vec![(spec.query_key, query.text.as_str())];
        if let Some(k) = spec.size_key {
            params.push((k, limit.as_str()));
        }
        params.extend_from_slice(spec.extra_params);
        let v: Value = http
            .get(spec.url)
            .query(&params)
            .header("Accept", "application/json")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(parse(spec, &v, query.limit))
    }
}

fn parse(spec: &RegistrySpec, v: &Value, max: usize) -> Vec<SearchResult> {
    let items = match v.pointer(spec.results_ptr).and_then(|x| x.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    let im = &spec.item;
    let mut out = Vec::new();
    for item in items {
        let name = item.pointer(im.name).and_then(|x| x.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let url = build_url(im, item, name);
        if url.is_empty() {
            continue;
        }
        let description = item
            .pointer(im.description)
            .and_then(|x| x.as_str())
            .map(collapse_ws)
            .unwrap_or_default();
        let version = im
            .version
            .and_then(|p| item.pointer(p))
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty());
        out.push(SearchResult {
            title: match version {
                Some(v) => format!("{name} {v}"),
                None => name.to_string(),
            },
            url,
            snippet: description,
            meta: version.map(|v| format!("v{v}")),
            ..Default::default()
        });
        if out.len() >= max {
            break;
        }
    }
    out
}

fn build_url(im: &ItemMap, item: &Value, name: &str) -> String {
    if let Some(ptr) = im.url_field {
        if let Some(u) = item.pointer(ptr).and_then(|x| x.as_str()) {
            if u.starts_with("http") {
                return u.to_string();
            }
            if !u.is_empty() {
                return format!("{}{u}", im.url_base);
            }
        }
    }
    im.url_template
        .map(|t| t.replace("{name}", name))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cratesio_parse_with_template_url_and_version() {
        let v = serde_json::json!({
            "crates": [
                {"name": "serde", "description": "  ser/de  ", "newest_version": "1.0.2"}
            ]
        });
        let out = parse(&cratesio::SPEC, &v, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "serde 1.0.2");
        assert_eq!(out[0].url, "https://crates.io/crates/serde");
        assert_eq!(out[0].snippet, "ser/de"); // collapse_ws trims
        assert_eq!(out[0].meta.as_deref(), Some("v1.0.2"));
    }

    #[test]
    fn npm_parse_with_url_field() {
        let v = serde_json::json!({
            "objects": [
                {"package": {"name": "left-pad", "description": "pad", "version": "1.3.0",
                             "links": {"npm": "https://www.npmjs.com/package/left-pad"}}}
            ]
        });
        let out = parse(&npm::SPEC, &v, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://www.npmjs.com/package/left-pad");
        assert_eq!(out[0].title, "left-pad 1.3.0");
    }

    #[test]
    fn mdn_parse_prefixes_relative_url() {
        let v = serde_json::json!({
            "documents": [
                {"title": "Array.map()", "summary": "maps", "mdn_url": "/en-US/docs/Web/JavaScript"}
            ]
        });
        let out = parse(&mdn::SPEC, &v, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].url,
            "https://developer.mozilla.org/en-US/docs/Web/JavaScript"
        );
        assert_eq!(out[0].title, "Array.map()");
        assert!(out[0].meta.is_none());
    }

    #[test]
    fn rubygems_parse_root_array() {
        // RubyGems returns a top-level array; the empty results pointer selects it.
        let v = serde_json::json!([
            {"name": "rails", "info": "Full-stack web framework", "version": "8.1.3"}
        ]);
        let out = parse(&rubygems::SPEC, &v, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://rubygems.org/gems/rails");
        assert_eq!(out[0].title, "rails 8.1.3");
        assert_eq!(out[0].snippet, "Full-stack web framework");
    }

    #[test]
    fn nuget_parse_data_with_id_key() {
        let v = serde_json::json!({
            "data": [
                {"@type": "Package", "id": "Newtonsoft.Json", "version": "13.0.3",
                 "description": "JSON for .NET"}
            ]
        });
        let out = parse(&nuget::SPEC, &v, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "Newtonsoft.Json 13.0.3");
        assert_eq!(out[0].url, "https://www.nuget.org/packages/Newtonsoft.Json");
    }

    #[test]
    fn hex_parse_nested_description() {
        let v = serde_json::json!([
            {"name": "phoenix", "meta": {"description": "Productive web framework"}}
        ]);
        let out = parse(&hex::SPEC, &v, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].url, "https://hex.pm/packages/phoenix");
        assert_eq!(out[0].snippet, "Productive web framework");
    }

    #[test]
    fn missing_results_array_is_empty() {
        assert!(parse(&cratesio::SPEC, &serde_json::json!({}), 10).is_empty());
    }
}
