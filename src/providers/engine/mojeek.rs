//! Mojeek engine spec — `www.mojeek.com/search`. Independent index, tolerant of
//! automation (a reliable fallback). No `site:` operator, so code search appends
//! the forge domains as keywords and filters the results to them.

use super::{CodeScope, EngineSpec, Extract, Method};

pub(super) static SPEC: EngineSpec = EngineSpec {
    id: "mojeek",
    url: "https://www.mojeek.com/search",
    method: Method::Get,
    extract: Extract::Selectors {
        link: "a.title",
        snippet: "p.s",
    },
    code_scope: CodeScope::Keyword,
    extra_params: &[],
};
