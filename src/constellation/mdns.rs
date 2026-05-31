//! Best-effort mDNS LAN discovery for the constellation. Advertises this node and
//! browses for peers running the same service, adding any it finds to the peer
//! set. Entirely optional: failures are logged and ignored, and it only runs
//! when `[network].mdns` is true.

use std::collections::HashMap;
use std::sync::Arc;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use super::Constellation;

const SERVICE_TYPE: &str = "_lodestone._tcp.local.";

/// Spawn the register + browse loop. `port` is the local HTTP port (used as the
/// advertised port unless `[network].advertise_port` overrides it).
pub(super) fn spawn(constellation: Arc<Constellation>, port: u16) {
    tokio::spawn(async move {
        if let Err(e) = run(constellation, port).await {
            tracing::warn!(error = %e, "mDNS discovery disabled (failed to start)");
        }
    });
}

async fn run(constellation: Arc<Constellation>, port: u16) -> anyhow::Result<()> {
    let daemon = ServiceDaemon::new()?;
    let node_id = constellation.node_id().to_string();
    let advertise_port = constellation.advertise_port(port);
    let host = format!("{node_id}.local.");

    let mut props = HashMap::new();
    props.insert("id".to_string(), node_id.clone());

    // Empty address + enable_addr_auto: the daemon fills in our LAN addresses.
    let info = ServiceInfo::new(SERVICE_TYPE, &node_id, &host, "", advertise_port, props)?
        .enable_addr_auto();
    daemon.register(info)?;
    tracing::info!(node_id = %node_id, port = advertise_port, "mDNS: advertising on the LAN");

    let receiver = daemon.browse(SERVICE_TYPE)?;
    while let Ok(event) = receiver.recv_async().await {
        if let ServiceEvent::ServiceResolved(svc) = event {
            let peer_port = svc.get_port();
            // Self-resolution: record every LAN address mDNS saw us on
            // so add_peer can refuse them later if a peer gossips one
            // back. Then skip the peer add.
            if svc.get_property_val_str("id") == Some(node_id.as_str()) {
                for ip in svc.get_addresses_v4() {
                    constellation.mark_local_url(&format!("http://{ip}:{peer_port}"));
                }
                continue;
            }
            for ip in svc.get_addresses_v4() {
                constellation.add_peer(&format!("http://{ip}:{peer_port}"));
            }
        }
    }
    Ok(())
}
