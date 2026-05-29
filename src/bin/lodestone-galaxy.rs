//! `lodestone-galaxy` — the standalone galaxy **broker**: a tiny, publicly-reachable
//! rendezvous directory that links lodestone *constellations* across networks. It is
//! NOT a proxy — it only stores `{ constellation → public ingress endpoint(s) }` so
//! constellations discover each other and then talk directly. This is a separate
//! program from `lodestone-mcp` (the MCP server + constellation); run it on a host
//! with an open/forwarded port.
//!
//! Config via env (or argv[1] for the bind address):
//!   LODESTONE_GALAXY_BIND      host:port to listen on   (default 0.0.0.0:8077)
//!   LODESTONE_GALAXY_TOKEN     optional shared secret for /galaxy/*  (default none)
//!   LODESTONE_GALAXY_TTL_SECS  evict a constellation after this idle (default 90)

#[path = "../galaxy/broker.rs"]
mod broker;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let bind = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("LODESTONE_GALAXY_BIND").ok())
        .unwrap_or_else(|| "0.0.0.0:8077".to_string());
    let token = std::env::var("LODESTONE_GALAXY_TOKEN").unwrap_or_default();
    let ttl_secs = std::env::var("LODESTONE_GALAXY_TTL_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(90);

    let b = broker::GalaxyBroker::new(&token, ttl_secs);
    let app = broker::galaxy_routes(b);

    let listener = match tokio::net::TcpListener::bind(&bind).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("lodestone-galaxy: failed to bind {bind}: {e}");
            std::process::exit(1);
        }
    };
    tracing::info!(
        token = if token.is_empty() { "none" } else { "set" },
        ttl_secs,
        "lodestone-galaxy broker listening on http://{bind}/galaxy"
    );
    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
    {
        eprintln!("lodestone-galaxy: server error: {e}");
        std::process::exit(1);
    }
}
