//! Packagist registry spec — the PHP/Composer package index. Keyless JSON search
//! at `/search.json?q=`; each result carries a ready `url`.

use super::{ItemMap, RegistrySpec};

pub(super) static SPEC: RegistrySpec = RegistrySpec {
    id: "packagist",
    url: "https://packagist.org/search.json",
    query_key: "q",
    size_key: Some("per_page"),
    extra_params: &[],
    results_ptr: "/results",
    item: ItemMap {
        name: "/name",
        description: "/description",
        url_field: Some("/url"),
        url_template: Some("https://packagist.org/packages/{name}"),
        url_base: "",
        version: None,
    },
};
