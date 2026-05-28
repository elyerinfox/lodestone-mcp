//! Bespoke providers — sources whose transport/parsing is unique enough that
//! they implement `SearchProvider` directly rather than fitting a spec-driven
//! family (see CONTRIBUTING, tier 3).

mod grep_app;
mod medium;
mod searxng;

pub(crate) use grep_app::GrepApp;
pub(crate) use medium::Medium;
pub(crate) use searxng::Searxng;
