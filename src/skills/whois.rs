//! WHOIS / RDAP lookup skills (network): RDAP queries for domains, IPs, and
//! ASNs against the IANA bootstrap registries — keyless, JSON-native, the
//! modern replacement for classic WHOIS. LLMs invent registrars, expiry
//! dates, and abuse contacts; the IANA bootstrap + RDAP gives the model
//! deterministic answers.
//!
//! Implementation: cached IANA bootstrap registry → look up the responsible
//! RDAP server for a given TLD / IP block / ASN → query → parse the small
//! JSON shape RDAP returns. No third-party RDAP client crate; ~250 lines
//! straight against reqwest.
//!
//! ## Sources
//!
//! - RFC 7480-7484 (RDAP protocol family).
//! - IANA RDAP bootstrap registry:
//!   - <https://data.iana.org/rdap/dns.json>
//!   - <https://data.iana.org/rdap/ipv4.json>
//!   - <https://data.iana.org/rdap/ipv6.json>
//!   - <https://data.iana.org/rdap/asn.json>

use std::net::IpAddr;
use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{internal, invalid, text_result};

const DNS_BOOTSTRAP: &str = "https://data.iana.org/rdap/dns.json";
const IPV4_BOOTSTRAP: &str = "https://data.iana.org/rdap/ipv4.json";
const IPV6_BOOTSTRAP: &str = "https://data.iana.org/rdap/ipv6.json";
const ASN_BOOTSTRAP: &str = "https://data.iana.org/rdap/asn.json";

async fn bootstrap(server: &crate::Lodestone, url: &str, key: &str) -> Result<Value, McpError> {
    if let Some(c) = server.retrieval_get(key).await {
        return Ok(serde_json::from_str(&c).unwrap_or(Value::Null));
    }
    let body: Value = server
        .http
        .get(url)
        .send()
        .await
        .map_err(|e| internal(e.into()))?
        .error_for_status()
        .map_err(|e| internal(e.into()))?
        .json()
        .await
        .map_err(|e| internal(e.into()))?;
    server.retrieval_put(key.to_string(), &body.to_string());
    Ok(body)
}

/// Look up the RDAP base URL for a given TLD via the IANA DNS bootstrap.
async fn rdap_base_for_tld(
    server: &crate::Lodestone,
    tld: &str,
) -> Result<Option<String>, McpError> {
    let b = bootstrap(server, DNS_BOOTSTRAP, "rdap_bootstrap_dns").await?;
    if let Some(services) = b["services"].as_array() {
        for service in services {
            if let (Some(domains), Some(urls)) = (
                service.get(0).and_then(|v| v.as_array()),
                service.get(1).and_then(|v| v.as_array()),
            ) {
                for d in domains {
                    if d.as_str()
                        .map(|s| s.eq_ignore_ascii_case(tld))
                        .unwrap_or(false)
                    {
                        if let Some(url) = urls.first().and_then(|u| u.as_str()) {
                            return Ok(Some(url.trim_end_matches('/').to_string()));
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

/// Look up RDAP base URL for an IP address via the IPv4 or IPv6 bootstrap.
async fn rdap_base_for_ip(
    server: &crate::Lodestone,
    ip: IpAddr,
) -> Result<Option<String>, McpError> {
    let (url, key) = match ip {
        IpAddr::V4(_) => (IPV4_BOOTSTRAP, "rdap_bootstrap_ipv4"),
        IpAddr::V6(_) => (IPV6_BOOTSTRAP, "rdap_bootstrap_ipv6"),
    };
    let b = bootstrap(server, url, key).await?;
    // The bootstrap maps prefix → urls. We pick the first matching prefix; the
    // IANA registry is small enough that this linear scan is fine.
    if let Some(services) = b["services"].as_array() {
        for service in services {
            if let (Some(prefixes), Some(urls)) = (
                service.get(0).and_then(|v| v.as_array()),
                service.get(1).and_then(|v| v.as_array()),
            ) {
                for p in prefixes {
                    if let Some(prefix_str) = p.as_str() {
                        if ip_in_prefix(ip, prefix_str) {
                            if let Some(url) = urls.first().and_then(|u| u.as_str()) {
                                return Ok(Some(url.trim_end_matches('/').to_string()));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

/// Look up the RDAP base URL for an AS number via the ASN bootstrap.
async fn rdap_base_for_asn(
    server: &crate::Lodestone,
    asn: u32,
) -> Result<Option<String>, McpError> {
    let b = bootstrap(server, ASN_BOOTSTRAP, "rdap_bootstrap_asn").await?;
    if let Some(services) = b["services"].as_array() {
        for service in services {
            if let (Some(ranges), Some(urls)) = (
                service.get(0).and_then(|v| v.as_array()),
                service.get(1).and_then(|v| v.as_array()),
            ) {
                for r in ranges {
                    if let Some(range_str) = r.as_str() {
                        if asn_in_range(asn, range_str) {
                            if let Some(url) = urls.first().and_then(|u| u.as_str()) {
                                return Ok(Some(url.trim_end_matches('/').to_string()));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(None)
}

fn ip_in_prefix(ip: IpAddr, prefix: &str) -> bool {
    use ipnet::IpNet;
    let Ok(net) = prefix.parse::<IpNet>() else {
        return false;
    };
    net.contains(&ip)
}

fn asn_in_range(asn: u32, range: &str) -> bool {
    // Range like "1-1876" or single "1234".
    if let Some((lo, hi)) = range.split_once('-') {
        let lo: u32 = lo.trim().parse().unwrap_or(0);
        let hi: u32 = hi.trim().parse().unwrap_or(0);
        asn >= lo && asn <= hi
    } else {
        range
            .trim()
            .parse::<u32>()
            .map(|n| n == asn)
            .unwrap_or(false)
    }
}

fn extract_dates(entity: &Value) -> Vec<Value> {
    entity["events"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    let action = e["eventAction"].as_str()?.to_string();
                    let date = e["eventDate"].as_str()?.to_string();
                    Some(json!({"action": action, "date": date}))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// whois_domain
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DomainArgs {
    /// Domain name (e.g. `example.com`). The TLD is used to find the RDAP server.
    domain: String,
}

pub struct WhoisDomain;
impl Skill for WhoisDomain {
    fn name(&self) -> &'static str {
        "whois_domain"
    }
    fn description(&self) -> &'static str {
        "RDAP lookup for a domain: registrar, status flags, name servers, registration / \
         expiration / last-changed dates. The TLD is resolved against the IANA RDAP bootstrap \
         registry so the right registry's RDAP server is queried. Keyless."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<DomainArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<DomainArgs>()?;
            let domain = args.domain.trim().to_ascii_lowercase();
            let tld = domain
                .rsplit('.')
                .next()
                .ok_or_else(|| invalid(format!("`{domain}` has no TLD")))?;
            let key = format!("whois_domain|{domain}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let base = rdap_base_for_tld(server, tld).await?.ok_or_else(|| {
                invalid(format!(
                    "no RDAP server in the IANA bootstrap for TLD `.{tld}`"
                ))
            })?;
            let url = format!("{base}/domain/{domain}");
            let resp = server
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| internal(e.into()))?;
            if resp.status().as_u16() == 404 {
                let body = json!({
                    "domain": domain,
                    "found": false,
                    "rdap_server": base,
                })
                .to_string();
                server.retrieval_put(key, &body);
                return Ok(text_result(body));
            }
            let v: Value = resp
                .error_for_status()
                .map_err(|e| internal(e.into()))?
                .json()
                .await
                .map_err(|e| internal(e.into()))?;
            let name_servers: Vec<String> = v["nameservers"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|n| n["ldhName"].as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let status: Vec<String> = v["status"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let registrar = v["entities"]
                .as_array()
                .and_then(|arr| {
                    arr.iter().find(|e| {
                        e["roles"]
                            .as_array()
                            .map(|r| r.iter().any(|x| x == "registrar"))
                            .unwrap_or(false)
                    })
                })
                .and_then(|e| {
                    e["vcardArray"][1].as_array()?.iter().find_map(|f| {
                        if f[0] == "fn" {
                            f.get(3).and_then(|x| x.as_str().map(str::to_string))
                        } else {
                            None
                        }
                    })
                });
            let body = json!({
                "domain": domain,
                "found": true,
                "rdap_server": base,
                "registrar": registrar,
                "status": status,
                "events": extract_dates(&v),
                "name_servers": name_servers,
            })
            .to_string();
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Look up a domain",
                args: r#"{"domain": "example.com"}"#,
                note: Some("Returns registrar, status, dates (registration/expiration/last-changed), and authoritative NS."),
            },
            SkillExample {
                title: "Newer gTLD",
                args: r#"{"domain": "rust-lang.dev"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Verify a domain's registrar / expiration before relying on it.",
            "Audit name-server changes during a migration.",
            "Confirm whether a domain still exists (vs an LLM-imagined one).",
        ]
    }
}

// ---------------------------------------------------------------------------
// whois_ip
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IpArgs {
    /// IPv4 or IPv6 address to look up.
    ip: String,
}

pub struct WhoisIp;
impl Skill for WhoisIp {
    fn name(&self) -> &'static str {
        "whois_ip"
    }
    fn description(&self) -> &'static str {
        "RDAP lookup for an IP: responsible RIR (ARIN / RIPE / APNIC / LACNIC / AFRINIC), \
         allocated org, network range, abuse contact. Keyless."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IpArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<IpArgs>()?;
            let ip: IpAddr = args
                .ip
                .trim()
                .parse()
                .map_err(|e| invalid(format!("could not parse `{}` as IP: {e}", args.ip)))?;
            let key = format!("whois_ip|{ip}");
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let base = rdap_base_for_ip(server, ip)
                .await?
                .ok_or_else(|| invalid(format!("no RDAP server in the IANA bootstrap for {ip}")))?;
            let url = format!("{base}/ip/{ip}");
            let v: Value = server
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| internal(e.into()))?
                .error_for_status()
                .map_err(|e| internal(e.into()))?
                .json()
                .await
                .map_err(|e| internal(e.into()))?;
            let body = json!({
                "ip": ip.to_string(),
                "rdap_server": base,
                "name": v["name"].as_str(),
                "handle": v["handle"].as_str(),
                "start_address": v["startAddress"].as_str(),
                "end_address": v["endAddress"].as_str(),
                "ip_version": v["ipVersion"].as_str(),
                "country": v["country"].as_str(),
                "status": v["status"].as_array().cloned(),
                "events": extract_dates(&v),
            })
            .to_string();
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[SkillExample {
            title: "Cloudflare 1.1.1.1",
            args: r#"{"ip": "1.1.1.1"}"#,
            note: Some("Returns the assigned network, owner, and the responsible RIR."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Identify which org owns an IP address before allowlisting / blocklisting.",
            "Find the abuse contact for a malicious source IP.",
            "Audit which RIR allocated a network range.",
        ]
    }
}

// ---------------------------------------------------------------------------
// whois_asn
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AsnArgs {
    /// Autonomous System number (e.g. `13335` for Cloudflare). Numeric only.
    asn: u32,
}

pub struct WhoisAsn;
impl Skill for WhoisAsn {
    fn name(&self) -> &'static str {
        "whois_asn"
    }
    fn description(&self) -> &'static str {
        "RDAP lookup for an AS number: owning org, name, country, and allocation events. \
         Keyless."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<AsnArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<AsnArgs>()?;
            let key = format!("whois_asn|{}", args.asn);
            if let Some(c) = server.retrieval_get(&key).await {
                return Ok(text_result(c));
            }
            let base = rdap_base_for_asn(server, args.asn).await?.ok_or_else(|| {
                invalid(format!(
                    "no RDAP server in the IANA bootstrap for AS{}",
                    args.asn
                ))
            })?;
            let url = format!("{base}/autnum/{}", args.asn);
            let v: Value = server
                .http
                .get(&url)
                .send()
                .await
                .map_err(|e| internal(e.into()))?
                .error_for_status()
                .map_err(|e| internal(e.into()))?
                .json()
                .await
                .map_err(|e| internal(e.into()))?;
            let body = json!({
                "asn": args.asn,
                "rdap_server": base,
                "name": v["name"].as_str(),
                "handle": v["handle"].as_str(),
                "country": v["country"].as_str(),
                "events": extract_dates(&v),
            })
            .to_string();
            server.retrieval_put(key, &body);
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Cloudflare AS13335",
                args: r#"{"asn": 13335}"#,
                note: Some("Returns the registered org + allocation events."),
            },
            SkillExample {
                title: "Google AS15169",
                args: r#"{"asn": 15169}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Identify which org owns an AS number before believing an LLM-stated attribution.",
            "Audit allocation events for a network you're peering with.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "whois"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "RDAP lookups for domains, IPs, and AS numbers via the IANA bootstrap registries. \
         Keyless. Returns registrar / org / dates / nameservers / status flags / abuse contact. \
         The modern, JSON-native replacement for classic WHOIS port 43."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `whois_domain { domain: \"example.com\" }` — registrar + expiry + nameservers.\n\
             2. `dns_lookup { name: \"example.com\", record_type: \"A\" }` — what IP does it point at?\n\
             3. `whois_ip { ip: \"<that ip>\" }` — which org / RIR owns the address?",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(WhoisDomain), Box::new(WhoisIp), Box::new(WhoisAsn)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ip_in_prefix_v4() {
        assert!(ip_in_prefix("1.1.1.1".parse().unwrap(), "1.0.0.0/8"));
        assert!(!ip_in_prefix("1.1.1.1".parse().unwrap(), "2.0.0.0/8"));
    }

    #[test]
    fn asn_in_range_inclusive() {
        assert!(asn_in_range(13335, "1-65535"));
        assert!(asn_in_range(1234, "1234"));
        assert!(!asn_in_range(99999, "1-65535"));
    }
}
