//! RubyGems registry spec — the Ruby package index. Keyless JSON search at
//! `/api/v1/search.json?query=`; the response is a top-level array (so the
//! results pointer is the empty/root pointer).

use super::{ItemMap, RegistrySpec};

pub(super) static SPEC: RegistrySpec = RegistrySpec {
    id: "rubygems",
    url: "https://rubygems.org/api/v1/search.json",
    query_key: "query",
    size_key: None,
    extra_params: &[],
    results_ptr: "",
    item: ItemMap {
        name: "/name",
        description: "/info",
        url_field: None,
        url_template: Some("https://rubygems.org/gems/{name}"),
        url_base: "",
        version: Some("/version"),
    },
};
