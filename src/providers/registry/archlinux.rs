//! Arch Linux official packages registry spec. Keyless JSON at
//! `/packages/search/json/?q=`; the canonical package URL is built from the
//! repo/arch/pkgname fields via JSON-pointer template placeholders.

use super::{ItemMap, RegistrySpec};

pub(super) static SPEC: RegistrySpec = RegistrySpec {
    id: "archlinux",
    url: "https://archlinux.org/packages/search/json/",
    query_key: "q",
    size_key: None,
    extra_params: &[],
    results_ptr: "/results",
    item: ItemMap {
        name: "/pkgname",
        description: "/pkgdesc",
        url_field: None,
        url_template: Some("https://archlinux.org/packages/{/repo}/{/arch}/{/pkgname}/"),
        url_base: "",
        version: Some("/pkgver"),
    },
};
