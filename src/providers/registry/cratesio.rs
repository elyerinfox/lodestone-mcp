//! crates.io registry spec — the Rust package index. Keyless JSON search at
//! `/api/v1/crates?q=`; result URLs are built from the crate name.

use super::{ItemMap, RegistrySpec};

pub(super) static SPEC: RegistrySpec = RegistrySpec {
    id: "cratesio",
    url: "https://crates.io/api/v1/crates",
    query_key: "q",
    size_key: Some("per_page"),
    extra_params: &[],
    results_ptr: "/crates",
    item: ItemMap {
        name: "/name",
        description: "/description",
        url_field: None,
        url_template: Some("https://crates.io/crates/{name}"),
        url_base: "",
        version: Some("/newest_version"),
    },
};
