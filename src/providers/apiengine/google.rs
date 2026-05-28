//! Google Programmable Search (Custom Search JSON API) spec (keyed). GET
//! `/customsearch/v1?q=` with `key` and `cx` query params; results under
//! `/items`. Active only when `[google_cse].key` and `.cx` are set.

use super::{ApiSpec, Auth};

pub(super) static SPEC: ApiSpec = ApiSpec {
    id: "google_cse",
    url: "https://www.googleapis.com/customsearch/v1",
    query_key: "q",
    size_key: Some("num"),
    size_cap: 10,
    auth: Auth::Query("key"),
    extra_params: &[],
    results_ptr: "/items",
    title: "/title",
    link: "/link",
    snippet: "/snippet",
};
