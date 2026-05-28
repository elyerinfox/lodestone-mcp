//! MDN Web Docs registry spec — Mozilla's web platform reference. Keyless JSON
//! search at `/api/v1/search?q=`; `mdn_url` is site-relative, so it's prefixed
//! with the MDN origin.

use super::{ItemMap, RegistrySpec};

pub(super) static SPEC: RegistrySpec = RegistrySpec {
    id: "mdn",
    url: "https://developer.mozilla.org/api/v1/search",
    query_key: "q",
    size_key: None,
    extra_params: &[],
    results_ptr: "/documents",
    item: ItemMap {
        name: "/title",
        description: "/summary",
        url_field: Some("/mdn_url"),
        url_template: None,
        url_base: "https://developer.mozilla.org",
        version: None,
    },
};
