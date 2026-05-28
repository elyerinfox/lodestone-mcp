//! Bespoke providers — sources whose transport/parsing is unique enough that
//! they implement `SearchProvider` directly rather than fitting a spec-driven
//! family (see CONTRIBUTING, tier 3).

mod grep_app;
mod medium;

pub(crate) use grep_app::GrepApp;
pub(crate) use medium::Medium;
