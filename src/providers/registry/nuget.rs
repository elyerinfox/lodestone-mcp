//! NuGet registry spec — the .NET package index. Keyless JSON search via the
//! NuGet search service (`/query?q=`); results live under `/data`, keyed by `id`.

use super::{ItemMap, RegistrySpec};

pub(super) static SPEC: RegistrySpec = RegistrySpec {
    id: "nuget",
    url: "https://azuresearch-usnc.nuget.org/query",
    query_key: "q",
    size_key: Some("take"),
    extra_params: &[],
    results_ptr: "/data",
    item: ItemMap {
        name: "/id",
        description: "/description",
        url_field: None,
        url_template: Some("https://www.nuget.org/packages/{name}"),
        url_base: "",
        version: Some("/version"),
    },
};
