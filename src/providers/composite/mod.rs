//! Composite providers — bespoke shells that pick one of several modes at
//! runtime (and often reuse a spec-driven family for one of them), honoring the
//! golden rules (see CONTRIBUTING, "Composite providers").

mod github;
mod stackexchange;

pub(crate) use github::Github;
pub(crate) use stackexchange::StackExchange;
