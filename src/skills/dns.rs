//! DNS skills (local + network): keyless DNS lookups across the major record
//! types, reverse PTR lookups, and a propagation check that fans out across
//! ~10 well-known public resolvers in parallel. Pure-Rust via the
//! `hickory-resolver` crate. LLMs hallucinate DNS records constantly — TXT
//! bodies, MX priorities, SPF policies — so deterministic lookups are the
//! right answer.
//!
//! ## Sources
//!
//! - RFC 1034 / 1035 (DNS base spec).
//! - RFC 4034 / 4035 / 6840 (DNSSEC).
//! - RFC 8499 (DNS terminology).

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use hickory_resolver::config::{NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::TokioResolver;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// Well-known public resolvers used by `dns_propagation`.
const PUBLIC_RESOLVERS: &[(&str, &str)] = &[
    ("Cloudflare", "1.1.1.1"),
    ("Cloudflare-2", "1.0.0.1"),
    ("Google", "8.8.8.8"),
    ("Google-2", "8.8.4.4"),
    ("Quad9", "9.9.9.9"),
    ("Quad9-2", "149.112.112.112"),
    ("OpenDNS", "208.67.222.222"),
    ("OpenDNS-2", "208.67.220.220"),
    ("NextDNS", "45.90.28.0"),
    ("Verisign", "64.6.64.6"),
];

fn make_resolver(target: Option<IpAddr>) -> Result<TokioResolver, McpError> {
    // Default to Cloudflare 1.1.1.1 for predictability across hosts.
    let ip = target.unwrap_or(IpAddr::from([1, 1, 1, 1]));
    let config = ResolverConfig::from_parts(None, vec![], vec![NameServerConfig::udp(ip)]);
    let mut builder = TokioResolver::builder_with_config(config, TokioRuntimeProvider::default());
    {
        let opts = builder.options_mut();
        opts.timeout = Duration::from_secs(3);
        opts.attempts = 1;
    }
    builder
        .build()
        .map_err(|e| invalid(format!("DNS resolver init failed: {e}")))
}

/// Build the reverse-DNS (`PTR`) query name for an IP address: `4.3.2.1.in-addr.arpa.`
/// for IPv4, the nibble-reversed `…ip6.arpa.` form for IPv6. hickory-resolver 0.26
/// dropped the convenience `reverse_lookup`, so we form the PTR name and look it up.
fn reverse_name(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            format!("{}.{}.{}.{}.in-addr.arpa.", o[3], o[2], o[1], o[0])
        }
        IpAddr::V6(v6) => {
            let mut s = String::with_capacity(72);
            for octet in v6.octets().iter().rev() {
                s.push_str(&format!("{:x}.{:x}.", octet & 0x0f, octet >> 4));
            }
            s.push_str("ip6.arpa.");
            s
        }
    }
}

fn parse_record_type(s: &str) -> Result<RecordType, McpError> {
    let up = s.trim().to_ascii_uppercase();
    match up.as_str() {
        "A" => Ok(RecordType::A),
        "AAAA" => Ok(RecordType::AAAA),
        "MX" => Ok(RecordType::MX),
        "TXT" => Ok(RecordType::TXT),
        "CNAME" => Ok(RecordType::CNAME),
        "NS" => Ok(RecordType::NS),
        "SOA" => Ok(RecordType::SOA),
        "SRV" => Ok(RecordType::SRV),
        "CAA" => Ok(RecordType::CAA),
        "PTR" => Ok(RecordType::PTR),
        "DS" => Ok(RecordType::DS),
        "DNSKEY" => Ok(RecordType::DNSKEY),
        other => Err(invalid(format!(
            "unsupported record type `{other}` (try A/AAAA/MX/TXT/CNAME/NS/SOA/SRV/CAA/PTR/DS/DNSKEY)"
        ))),
    }
}

fn parse_resolver(s: Option<&str>) -> Result<Option<IpAddr>, McpError> {
    match s.map(str::trim).filter(|x| !x.is_empty()) {
        None => Ok(None),
        Some(raw) => raw
            .parse::<IpAddr>()
            .map(Some)
            .map_err(|e| invalid(format!("could not parse resolver `{raw}` as IP: {e}"))),
    }
}

// ---------------------------------------------------------------------------
// dns_lookup
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct LookupArgs {
    /// FQDN to look up, e.g. `example.com`.
    name: String,
    /// Record type. Default `A`. Supported: A, AAAA, MX, TXT, CNAME, NS,
    /// SOA, SRV, CAA, PTR, DS, DNSKEY.
    #[serde(default)]
    record_type: Option<String>,
    /// Optional resolver IP override (e.g. `"1.1.1.1"`). Defaults to
    /// Cloudflare (1.1.1.1) for predictability across hosts.
    #[serde(default)]
    resolver: Option<String>,
}

pub struct DnsLookup;
impl Skill for DnsLookup {
    fn name(&self) -> &'static str {
        "dns_lookup"
    }
    fn description(&self) -> &'static str {
        "Resolve a DNS name against a chosen record type (A, AAAA, MX, TXT, CNAME, NS, SOA, SRV, \
         CAA, PTR, DS, DNSKEY). Defaults to Cloudflare 1.1.1.1 for predictability; override with \
         `resolver` to test a specific server. Keyless, plain DNS protocol."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<LookupArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<LookupArgs>()?;
            let rt = parse_record_type(args.record_type.as_deref().unwrap_or("A"))?;
            let resolver_ip = parse_resolver(args.resolver.as_deref())?;
            let resolver = make_resolver(resolver_ip)?;
            let records = resolver
                .lookup(args.name.as_str(), rt)
                .await
                .map_err(|e| invalid(format!("DNS lookup failed: {e}")))?;
            let answers: Vec<String> =
                records.answers().iter().map(|r| format!("{}", r.data)).collect();
            Ok(text_result(
                json!({
                    "name": args.name,
                    "record_type": format!("{rt:?}"),
                    "resolver": resolver_ip.map(|ip| ip.to_string()).unwrap_or_else(|| "1.1.1.1 (default)".into()),
                    "answer_count": answers.len(),
                    "answers": answers,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "A records (default)",
                args: r#"{"name": "example.com"}"#,
                note: Some("Looks up A records via Cloudflare 1.1.1.1."),
            },
            SkillExample {
                title: "MX records",
                args: r#"{"name": "github.com", "record_type": "MX"}"#,
                note: Some("Returns mail-exchange records with priority + host."),
            },
            SkillExample {
                title: "TXT (often where SPF / DMARC / verification tokens live)",
                args: r#"{"name": "google.com", "record_type": "TXT"}"#,
                note: None,
            },
            SkillExample {
                title: "Override resolver",
                args: r#"{"name": "example.com", "record_type": "A", "resolver": "8.8.8.8"}"#,
                note: Some("Forces the query through Google Public DNS."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Confirm a domain's actual SPF / DMARC / DKIM TXT records without guessing.",
            "Verify MX records before configuring mail flow.",
            "Compare what two resolvers return for the same name.",
        ]
    }
}

// ---------------------------------------------------------------------------
// dns_reverse
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReverseArgs {
    /// IPv4 or IPv6 address to PTR-lookup.
    ip: String,
    /// Optional resolver IP override.
    #[serde(default)]
    resolver: Option<String>,
}

pub struct DnsReverse;
impl Skill for DnsReverse {
    fn name(&self) -> &'static str {
        "dns_reverse"
    }
    fn description(&self) -> &'static str {
        "Reverse-DNS lookup: given an IPv4 or IPv6 address, return the PTR record (often a host \
         name like `mail.example.com`). No PTR set returns an empty answer list, not an error."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ReverseArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::Shared {
            source: crate::constellation::Source::Other,
        }
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ReverseArgs>()?;
            let ip: IpAddr = args
                .ip
                .trim()
                .parse()
                .map_err(|e| invalid(format!("could not parse `{}` as IP: {e}", args.ip)))?;
            let resolver_ip = parse_resolver(args.resolver.as_deref())?;
            let resolver = make_resolver(resolver_ip)?;
            let names = match resolver.lookup(reverse_name(ip), RecordType::PTR).await {
                Ok(lk) => lk
                    .answers()
                    .iter()
                    .map(|r| r.data.to_string())
                    .collect::<Vec<_>>(),
                Err(_) => Vec::new(),
            };
            Ok(text_result(
                json!({
                    "ip": ip.to_string(),
                    "ptr_count": names.len(),
                    "ptr_records": names,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Reverse-lookup Cloudflare's own resolver",
                args: r#"{"ip": "1.1.1.1"}"#,
                note: Some("Returns `one.one.one.one.` or similar."),
            },
            SkillExample {
                title: "IPv6 reverse",
                args: r#"{"ip": "2606:4700:4700::1111"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Identify what host an IP address belongs to (often used in log enrichment).",
            "Confirm a mail server's forward+reverse DNS match for SPF posture.",
        ]
    }
}

// ---------------------------------------------------------------------------
// dns_propagation
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PropagationArgs {
    /// FQDN to query.
    name: String,
    /// Record type. Default `A`.
    #[serde(default)]
    record_type: Option<String>,
}

pub struct DnsPropagation;
impl Skill for DnsPropagation {
    fn name(&self) -> &'static str {
        "dns_propagation"
    }
    fn description(&self) -> &'static str {
        "Fan out the same DNS query across ~10 well-known public resolvers (Cloudflare, Google, \
         Quad9, OpenDNS, NextDNS, Verisign — IPv4 anycast) in parallel and surface disagreements. \
         Each resolver row reports the answer set (or an error). Useful for confirming a recent \
         DNS change has fully propagated."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PropagationArgs>()
    }
    fn retrieval_policy(&self) -> crate::skills::RetrievalPolicy {
        crate::skills::RetrievalPolicy::None
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<PropagationArgs>()?;
            let rt = parse_record_type(args.record_type.as_deref().unwrap_or("A"))?;
            let mut futures: Vec<_> = Vec::new();
            for (label, ip) in PUBLIC_RESOLVERS {
                let name = args.name.clone();
                let label = *label;
                let ip = *ip;
                futures.push(tokio::spawn(async move {
                    let resolver_ip: IpAddr = ip.parse().unwrap();
                    let resolver = match make_resolver(Some(resolver_ip)) {
                        Ok(r) => r,
                        Err(e) => return (label, ip, Err(format!("{e}"))),
                    };
                    match resolver.lookup(name.as_str(), rt).await {
                        Ok(records) => {
                            let answers: Vec<String> =
                                records.answers().iter().map(|r| format!("{}", r.data)).collect();
                            (label, ip, Ok(answers))
                        }
                        Err(e) => (label, ip, Err(format!("{e}"))),
                    }
                }));
            }
            let mut rows: Vec<serde_json::Value> = Vec::new();
            for handle in futures {
                let (label, ip, result) = handle
                    .await
                    .map_err(|e| invalid(format!("task join failed: {e}")))?;
                match result {
                    Ok(answers) => rows.push(json!({
                        "resolver": label,
                        "resolver_ip": ip,
                        "answers": answers,
                    })),
                    Err(e) => rows.push(json!({
                        "resolver": label,
                        "resolver_ip": ip,
                        "error": e,
                    })),
                }
            }
            // Determine consensus by collecting unique sorted answer sets.
            let mut sets: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::new();
            for r in &rows {
                if let Some(a) = r.get("answers").and_then(|v| v.as_array()) {
                    let mut sorted: Vec<String> = a
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect();
                    sorted.sort();
                    *sets.entry(sorted.join(", ")).or_default() += 1;
                }
            }
            Ok(text_result(
                json!({
                    "name": args.name,
                    "record_type": format!("{rt:?}"),
                    "resolvers": rows,
                    "distinct_answer_sets": sets.len(),
                    "answer_set_counts": sets,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Check A-record propagation",
                args: r#"{"name": "example.com", "record_type": "A"}"#,
                note: Some("All resolvers should agree for a stable record."),
            },
            SkillExample {
                title: "Check MX during a mail-flow migration",
                args: r#"{"name": "yourdomain.test", "record_type": "MX"}"#,
                note: Some("`distinct_answer_sets > 1` means at least one resolver still has the old MX cached."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Confirm a recent DNS change has propagated globally before flipping a cutover.",
            "Spot-check whether a specific resolver is serving stale data.",
            "Audit consistency of a high-stakes record (e.g. NS, MX) across major providers.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "dns"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "Keyless DNS lookups (A / AAAA / MX / TXT / CNAME / NS / SOA / SRV / CAA / PTR / DS / \
         DNSKEY), reverse PTR queries, and a parallel propagation check across ~10 well-known \
         public resolvers. Backed by hickory-resolver."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `dns_lookup { name: \"example.com\", record_type: \"A\" }` — current A record.\n\
             2. `dns_lookup { name: \"example.com\", record_type: \"MX\" }` — mail-flow path.\n\
             3. `dns_propagation { name: \"example.com\", record_type: \"A\" }` — confirm every major resolver agrees.",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(DnsLookup),
        Box::new(DnsReverse),
        Box::new(DnsPropagation),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_record_types() {
        assert_eq!(parse_record_type("A").unwrap(), RecordType::A);
        assert_eq!(parse_record_type("mx").unwrap(), RecordType::MX);
        assert_eq!(parse_record_type("TXT").unwrap(), RecordType::TXT);
        assert!(parse_record_type("bogus").is_err());
    }

    #[test]
    fn parses_resolver_ip() {
        assert_eq!(
            parse_resolver(Some("8.8.8.8")).unwrap(),
            Some("8.8.8.8".parse().unwrap())
        );
        assert_eq!(parse_resolver(None).unwrap(), None);
        assert_eq!(parse_resolver(Some("")).unwrap(), None);
        assert!(parse_resolver(Some("not-an-ip")).is_err());
    }

    #[test]
    fn builds_reverse_name() {
        assert_eq!(
            reverse_name("1.1.1.1".parse().unwrap()),
            "1.1.1.1.in-addr.arpa."
        );
        assert_eq!(
            reverse_name("8.8.4.4".parse().unwrap()),
            "4.4.8.8.in-addr.arpa."
        );
        assert!(reverse_name("2606:4700:4700::1111".parse().unwrap()).ends_with(".ip6.arpa."));
    }
}
