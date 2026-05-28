//! Docker Hub registry spec — container image search. Keyless JSON at
//! `/v2/search/repositories/?query=`; results are under `/results`.

use super::{ItemMap, RegistrySpec};

pub(super) static SPEC: RegistrySpec = RegistrySpec {
    id: "dockerhub",
    url: "https://hub.docker.com/v2/search/repositories/",
    query_key: "query",
    size_key: Some("page_size"),
    extra_params: &[],
    results_ptr: "/results",
    item: ItemMap {
        name: "/repo_name",
        description: "/short_description",
        url_field: None,
        url_template: Some("https://hub.docker.com/r/{name}"),
        url_base: "",
        version: None,
    },
};
