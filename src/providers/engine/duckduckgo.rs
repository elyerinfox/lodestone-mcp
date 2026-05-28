//! DuckDuckGo engine spec — the `lite.duckduckgo.com` HTML endpoint. Honors
//! `site:`, so code search scopes precisely. Rate-limits aggressively by IP, so
//! it's typically paired with a more tolerant fallback (Mojeek).

use super::{CodeScope, EngineSpec, Extract, Method};

pub(super) static SPEC: EngineSpec = EngineSpec {
    id: "duckduckgo",
    url: "https://lite.duckduckgo.com/lite/",
    method: Method::PostForm,
    extract: Extract::Selectors {
        link: "a.result-link",
        snippet: "td.result-snippet",
    },
    code_scope: CodeScope::SiteOperator,
    extra_params: &[],
};
