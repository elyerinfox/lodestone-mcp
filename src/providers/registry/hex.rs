//! Hex registry spec — the Elixir/Erlang package index. Keyless JSON search at
//! `/api/packages?search=`; the response is a top-level array and the description
//! is nested under `/meta`.

use super::{ItemMap, RegistrySpec};

pub(super) static SPEC: RegistrySpec = RegistrySpec {
    id: "hex",
    url: "https://hex.pm/api/packages",
    query_key: "search",
    size_key: Some("per_page"),
    extra_params: &[],
    results_ptr: "",
    item: ItemMap {
        name: "/name",
        description: "/meta/description",
        url_field: None,
        url_template: Some("https://hex.pm/packages/{name}"),
        url_base: "",
        version: None,
    },
};
