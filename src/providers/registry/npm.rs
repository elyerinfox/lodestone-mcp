//! npm registry spec — the Node package index. Keyless JSON search at
//! `registry.npmjs.org/-/v1/search?text=`; each object nests its package under
//! `/package`, with a ready `links.npm` URL.

use super::{ItemMap, RegistrySpec};

pub(super) static SPEC: RegistrySpec = RegistrySpec {
    id: "npm",
    url: "https://registry.npmjs.org/-/v1/search",
    query_key: "text",
    size_key: Some("size"),
    extra_params: &[],
    results_ptr: "/objects",
    item: ItemMap {
        name: "/package/name",
        description: "/package/description",
        url_field: Some("/package/links/npm"),
        url_template: Some("https://www.npmjs.com/package/{name}"),
        url_base: "",
        version: Some("/package/version"),
    },
};
