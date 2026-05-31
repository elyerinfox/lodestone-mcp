//! Runtime control of the tracing subscriber.
//!
//! `main` builds a `tracing_subscriber::reload::Layer` around the
//! `EnvFilter` and parks the resulting handle here. The settings drawer
//! then calls [`set_level`] to swap the filter without restarting.
//! Level changes apply on the next emitted event — there is no buffer
//! to flush.
//!
//! Allowed levels are the five `tracing` ones — anything else is
//! rejected so a typo in the dashboard can't silently disable logging.

use std::sync::OnceLock;

use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::reload;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Registry;

type Reloader = reload::Handle<EnvFilter, Registry>;

static RELOAD: OnceLock<Reloader> = OnceLock::new();
static CURRENT: OnceLock<std::sync::Mutex<String>> = OnceLock::new();

/// Install the subscriber. Call exactly once at startup, before any
/// other tracing setup. Equivalent in behavior to the previous
/// `tracing_subscriber::fmt().with_env_filter(...).init()` line, but
/// the resulting filter is reloadable. Returns the initial level
/// string so the caller can echo it into the WS snapshot.
pub fn init() -> String {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let initial = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("lodestone_mcp=info,rmcp=warn"));
    let initial_str = initial.to_string();
    let (filter, handle) = reload::Layer::new(initial);
    // Best-effort: if init has already run (only really happens in
    // tests), keep going with the prior subscriber rather than panic.
    let _ = Registry::default()
        .with(filter)
        .with(fmt::layer())
        .try_init();
    let _ = RELOAD.set(handle);
    let _ = CURRENT.set(std::sync::Mutex::new(initial_str.clone()));
    initial_str
}

/// Snapshot the active filter string. Used by the WS snapshot so the
/// dashboard's level dropdown reflects what's actually in effect.
pub fn current() -> String {
    CURRENT
        .get()
        .map(|m| m.lock().unwrap().clone())
        .unwrap_or_default()
}

/// Apply a new level. Accepts the five tracing keywords (case-
/// insensitive). Anything else is rejected with `Err`. The crate
/// targets in the resulting filter mirror the startup default so a
/// switch to `debug` doesn't drown the operator in rmcp internals
/// they didn't ask for.
pub fn set_level(level: &str) -> Result<String, String> {
    let normalized = level.trim().to_ascii_lowercase();
    let lf = match normalized.as_str() {
        "off" => LevelFilter::OFF,
        "error" => LevelFilter::ERROR,
        "warn" => LevelFilter::WARN,
        "info" => LevelFilter::INFO,
        "debug" => LevelFilter::DEBUG,
        "trace" => LevelFilter::TRACE,
        other => return Err(format!("unknown level: {other}")),
    };
    let directive = if lf >= LevelFilter::DEBUG {
        format!("lodestone_mcp={normalized},rmcp=warn,hyper=info")
    } else {
        format!("lodestone_mcp={normalized},rmcp=warn")
    };
    let filter = EnvFilter::try_new(&directive).map_err(|e| e.to_string())?;
    let handle = RELOAD
        .get()
        .ok_or_else(|| "tracing not initialized".to_string())?;
    handle.reload(filter).map_err(|e| e.to_string())?;
    if let Some(m) = CURRENT.get() {
        *m.lock().unwrap() = directive.clone();
    }
    Ok(directive)
}
