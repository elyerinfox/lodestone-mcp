//! SSRF guard for delegated browser work (#130).
//!
//! When a constellation peer asks us to drive our browser, we must
//! refuse navigation to anything that resolves to the host's local
//! network — that's the classic SSRF attack: a peer using our browser
//! to enumerate our LAN, hit private admin panels, or fingerprint
//! cloud-metadata endpoints (169.254.169.254). This module gives the
//! browser session manager one entry point — [`assert_public`] — that
//! is called before every navigation on a restricted session.
//!
//! Strategy:
//! 1. Parse the URL. Refuse unsupported schemes (`file:`, `chrome:`,
//!    `about:` other than blank, `data:` past a small limit).
//! 2. Refuse hostnames that match well-known local TLDs (`.local`,
//!    `.internal`, `.lan`, `.intranet`, `.home.arpa`, `.test`).
//! 3. If the host is a literal IP, refuse it directly when it lands
//!    in any private / loopback / link-local / ULA range — no DNS
//!    needed, no TOCTOU window.
//! 4. Otherwise resolve the host via the system resolver and refuse
//!    if ANY resolved address is in the local set. We check every
//!    address Chromium might fall back to (the resolver returns a
//!    rotating list).
//!
//! Local-origin browser tools (the node's own MCP client) bypass this
//! check entirely — the manager opens an unrestricted session by
//! default and the model can browse anywhere. The guard fires only on
//! sessions explicitly marked `restrict_to_public = true`, which is
//! exclusively what `/constellation/browser_pool` does (see #128).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use rmcp::ErrorData as McpError;
use url::Url;

use crate::invalid;

/// Local TLDs we refuse without even resolving. `.local` is the
/// canonical mDNS one. Add to this set if you observe a leak.
const LOCAL_TLDS: &[&str] = &[
    ".local",
    ".internal",
    ".lan",
    ".intranet",
    ".home.arpa",
    ".test",
    ".invalid",
    ".localhost",
];

/// Schemes we accept on a restricted session. `about:blank` is fine
/// (it's the default tab content) but anything else under `about:`,
/// `chrome:`, or `file:` is refused outright.
fn is_restricted_scheme(scheme: &str) -> bool {
    matches!(scheme, "http" | "https")
}

/// Refuse the URL if it would expose the host's local network.
/// Returns Ok(()) when the URL passes; returns an McpError invalid-
/// request otherwise. Synchronous fast checks first; only does a DNS
/// lookup for hostnames that survive them.
pub async fn assert_public(url_str: &str) -> Result<(), McpError> {
    let url = match Url::parse(url_str) {
        Ok(u) => u,
        Err(e) => return Err(invalid(format!("invalid url: {e}"))),
    };

    // Accept the about:blank special case so the model can return a
    // restricted session to a known-empty state without hitting DNS.
    if url.as_str() == "about:blank" {
        return Ok(());
    }
    if !is_restricted_scheme(url.scheme()) {
        return Err(invalid(format!(
            "scheme {:?} is not permitted on a delegated browser session — only http/https",
            url.scheme()
        )));
    }
    let host_str = match url.host_str() {
        Some(h) => h,
        None => return Err(invalid("url has no host".to_string())),
    };
    let lower = host_str.to_ascii_lowercase();
    for tld in LOCAL_TLDS {
        if lower.ends_with(tld) || lower == tld.trim_start_matches('.') {
            return Err(invalid(format!(
                "host {host_str:?} resolves under a local TLD — refused for delegated browser work"
            )));
        }
    }
    // Literal IPs are decided synchronously — no DNS, no TOCTOU.
    if let Ok(ip) = host_str.parse::<IpAddr>() {
        if !is_public(&ip) {
            return Err(invalid(format!(
                "host {host_str} is in a private / loopback / link-local range — refused for \
                 delegated browser work"
            )));
        }
        return Ok(());
    }
    // Hostname: resolve and refuse if ANY result is local. Chromium
    // would fall back among them, so one private address poisons the
    // set. The host part of an http URL has no port suffix, so we
    // synthesize one for `lookup_host`.
    let lookup_target = format!("{host_str}:80");
    let addrs = tokio::net::lookup_host(lookup_target)
        .await
        .map_err(|e| invalid(format!("could not resolve host {host_str:?}: {e}")))?;
    for addr in addrs {
        if !is_public(&addr.ip()) {
            return Err(invalid(format!(
                "host {host_str:?} resolves to {} (private / loopback / link-local) — refused \
                 for delegated browser work",
                addr.ip()
            )));
        }
    }
    Ok(())
}

/// Is the IP address publicly routable? Refuses every special-purpose
/// range commented in the source. The set mirrors IANA's "Special-
/// Purpose Address Registry" entries you'd never want a browser to
/// reach when driven by an untrusted caller.
fn is_public(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    // Loopback 127.0.0.0/8, "any" 0.0.0.0/8.
    if o[0] == 127 || o[0] == 0 {
        return false;
    }
    // Private RFC1918.
    if o[0] == 10 {
        return false;
    }
    if o[0] == 172 && (16..=31).contains(&o[1]) {
        return false;
    }
    if o[0] == 192 && o[1] == 168 {
        return false;
    }
    // Link-local 169.254.0.0/16 (includes cloud-metadata 169.254.169.254).
    if o[0] == 169 && o[1] == 254 {
        return false;
    }
    // CGNAT 100.64.0.0/10 — refusal is conservative; you can carve out
    // by removing this if your deployment uses CGNAT addresses for
    // production traffic.
    if o[0] == 100 && (64..=127).contains(&o[1]) {
        return false;
    }
    // 192.0.0.0/24 (IETF protocol assignments) + 192.0.2.0/24 (TEST-NET-1).
    if o[0] == 192 && o[1] == 0 {
        return false;
    }
    // 198.18.0.0/15 (benchmarking) + 198.51.100.0/24 (TEST-NET-2).
    if o[0] == 198 && (o[1] == 18 || o[1] == 19 || o[1] == 51) {
        return false;
    }
    // 203.0.113.0/24 (TEST-NET-3).
    if o[0] == 203 && o[1] == 0 && o[2] == 113 {
        return false;
    }
    // 224.0.0.0/4 multicast, 240.0.0.0/4 reserved.
    if o[0] >= 224 {
        return false;
    }
    true
}

fn is_public_v6(ip: &Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return false;
    }
    let segments = ip.segments();
    // ULA fc00::/7.
    if segments[0] & 0xfe00 == 0xfc00 {
        return false;
    }
    // Link-local fe80::/10.
    if segments[0] & 0xffc0 == 0xfe80 {
        return false;
    }
    // IPv4-mapped: defer to the v4 check.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(&v4);
    }
    // Discard 100::/64 (RFC 6666).
    if segments[0] == 0x0100 && segments[1] == 0 && segments[2] == 0 && segments[3] == 0 {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refuses_loopback_literal() {
        assert!(assert_public("http://127.0.0.1/").await.is_err());
        assert!(assert_public("http://[::1]/").await.is_err());
    }

    #[tokio::test]
    async fn refuses_rfc1918_literal() {
        assert!(assert_public("http://10.0.0.5/").await.is_err());
        assert!(assert_public("http://172.16.0.1/").await.is_err());
        assert!(assert_public("http://192.168.1.1/").await.is_err());
    }

    #[tokio::test]
    async fn refuses_link_local() {
        assert!(assert_public("http://169.254.169.254/").await.is_err());
        assert!(assert_public("http://[fe80::1]/").await.is_err());
    }

    #[tokio::test]
    async fn refuses_local_tld() {
        assert!(assert_public("http://machine.local/").await.is_err());
        assert!(assert_public("http://router.lan/").await.is_err());
    }

    #[tokio::test]
    async fn refuses_unsupported_scheme() {
        assert!(assert_public("file:///etc/passwd").await.is_err());
        assert!(assert_public("chrome://settings/").await.is_err());
    }

    #[tokio::test]
    async fn allows_about_blank() {
        assert!(assert_public("about:blank").await.is_ok());
    }

    #[tokio::test]
    async fn allows_public_v4_literal() {
        assert!(assert_public("http://8.8.8.8/").await.is_ok());
    }
}
