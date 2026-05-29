//! The galaxy — linking *constellations* across networks.
//!
//! Split into two roles that live in different binaries:
//!
//! * **broker** ([`broker`]) — the rendezvous directory. It runs as a **separate
//!   program**, `lodestone-galaxy` (`src/bin/lodestone-galaxy.rs`), which includes
//!   `broker.rs` directly. The main `lodestone-mcp` binary does *not* embed it: the
//!   MCP server and its constellation are the main app; running a broker is its own
//!   process on a publicly-reachable host.
//! * **client** ([`client`]) — the participating side, embedded in the main app. It
//!   registers this constellation's public ingress endpoints with brokers and pulls
//!   their directories, adding other constellations as peers.
//!
//! The broker is deliberately **not a proxy**: it only stores/returns endpoints, so
//! constellations talk directly over `/constellation/*`. Galaxy connectivity is
//! entirely optional — a constellation is fully functional without any broker.

pub mod client;
