//! Brave Search API spec (keyed). GET `/res/v1/web/search?q=` with the
//! subscription token in the `X-Subscription-Token` header; results under
//! `/web/results`. Active only when `[brave].key` is set.

use super::{ApiSpec, Auth};

pub(super) static SPEC: ApiSpec = ApiSpec {
    id: "brave",
    url: "https://api.search.brave.com/res/v1/web/search",
    query_key: "q",
    size_key: Some("count"),
    size_cap: 20,
    auth: Auth::Header("X-Subscription-Token"),
    extra_params: &[],
    results_ptr: "/web/results",
    title: "/title",
    link: "/url",
    snippet: "/description",
};
