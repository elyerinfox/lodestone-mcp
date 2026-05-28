//! AUR (Arch User Repository) registry spec. Keyless JSON search via the AUR RPC
//! (`/rpc/?v=5&type=search&arg=`); results are under `/results`.

use super::{ItemMap, RegistrySpec};

pub(super) static SPEC: RegistrySpec = RegistrySpec {
    id: "aur",
    url: "https://aur.archlinux.org/rpc/",
    query_key: "arg",
    size_key: None,
    extra_params: &[("v", "5"), ("type", "search")],
    results_ptr: "/results",
    item: ItemMap {
        name: "/Name",
        description: "/Description",
        url_field: None,
        url_template: Some("https://aur.archlinux.org/packages/{name}"),
        url_base: "",
        version: Some("/Version"),
    },
};
