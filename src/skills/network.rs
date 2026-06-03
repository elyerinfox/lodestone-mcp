//! Network skills (local compute): CIDR / subnet math, IP classification, and
//! CIDR-list aggregation. Pure-Rust, no external network access — every tool
//! takes textual inputs (`"10.0.0.0/24"`, `"192.168.1.7"`) and returns
//! structured results. LLMs are notoriously bad at binary subnet arithmetic;
//! these tools give the model a deterministic answer for problems like
//! "what's the broadcast address of 10.42.7.83/19?" or "split 192.168.1.0/24
//! into four /26s".
//!
//! ## Sources
//!
//! - RFC 791 (IPv4), RFC 4632 (CIDR), RFC 1918 (private IPv4 ranges).
//! - IANA Special-Purpose Address Registry — what `net_ip_classify` recognizes
//!   as reserved / loopback / documentation / TEREDO / 6to4 / etc. Pinned to
//!   the May 2024 registry snapshot encoded in [`ipnet`] + this module's
//!   classifier.
//! - RFC 6598 (CGNAT 100.64.0.0/10), RFC 5737 / 3849 (documentation prefixes).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;

use futures::future::BoxFuture;
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx, SkillExample};
use crate::{invalid, text_result};

// ---------------------------------------------------------------------------
// net_cidr_info
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CidrArgs {
    /// CIDR notation (IPv4 or IPv6), e.g. `10.0.0.0/24` or `2001:db8::/32`.
    /// A bare IP without `/` is treated as a host route (`/32` or `/128`).
    cidr: String,
}

pub struct NetCidrInfo;
impl Skill for NetCidrInfo {
    fn name(&self) -> &'static str {
        "net_cidr_info"
    }
    fn description(&self) -> &'static str {
        "Decompose a CIDR block (IPv4 or IPv6) into network address, broadcast (v4), wildcard \
         mask, netmask, host range, total addresses, usable hosts, and prefix length. Pure local \
         compute via the `ipnet` crate; no network access."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CidrArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<CidrArgs>()?;
            let net = parse_cidr(&args.cidr)?;
            Ok(text_result(cidr_info(&net)))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "IPv4 /24",
                args: r#"{"cidr": "10.0.0.0/24"}"#,
                note: Some("Network 10.0.0.0, broadcast 10.0.0.255, 254 usable hosts."),
            },
            SkillExample {
                title: "IPv4 odd prefix (host inside)",
                args: r#"{"cidr": "10.42.7.83/19"}"#,
                note: Some("Returns the canonical network (10.42.0.0/19) plus broadcast 10.42.31.255 and host range."),
            },
            SkillExample {
                title: "IPv6 /32",
                args: r#"{"cidr": "2001:db8::/32"}"#,
                note: Some("IPv6 has no broadcast; the field is omitted for v6."),
            },
            SkillExample {
                title: "Bare IP treated as /32",
                args: r#"{"cidr": "192.0.2.1"}"#,
                note: Some("A bare v4 address becomes a /32 host route; bare v6 becomes a /128."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compute the broadcast / netmask / host range of a CIDR block without binary arithmetic.",
            "Confirm an off-aligned address (e.g. 10.42.7.83/19) belongs to its canonical network.",
            "Get the host-count / usable-host figures for capacity planning.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "cidr",
            min: Some(1),
            max: None,
        }]
    }
}

fn cidr_info(net: &IpNet) -> String {
    match net {
        IpNet::V4(v4) => {
            let mask = v4.netmask();
            let wildcard = invert_v4(mask);
            let net_addr = v4.network();
            let bcast = v4.broadcast();
            let total = 1u64 << (32 - v4.prefix_len());
            let usable = total.saturating_sub(2);
            let host_min = if v4.prefix_len() >= 31 {
                net_addr
            } else {
                Ipv4Addr::from(u32::from(net_addr).saturating_add(1))
            };
            let host_max = if v4.prefix_len() >= 31 {
                bcast
            } else {
                Ipv4Addr::from(u32::from(bcast).saturating_sub(1))
            };
            json!({
                "family": "IPv4",
                "network": net_addr.to_string(),
                "broadcast": bcast.to_string(),
                "netmask": mask.to_string(),
                "wildcard_mask": wildcard.to_string(),
                "prefix_len": v4.prefix_len(),
                "total_addresses": total,
                "usable_hosts": usable,
                "host_range": [host_min.to_string(), host_max.to_string()],
            })
            .to_string()
        }
        IpNet::V6(v6) => {
            let mask = v6.netmask();
            let net_addr = v6.network();
            let last_addr = v6.broadcast(); // crate's `broadcast` returns the all-ones address; conceptually the last host.
            let total = if v6.prefix_len() >= 64 {
                Some(1u128 << (128 - v6.prefix_len()))
            } else {
                None
            };
            let mut obj = json!({
                "family": "IPv6",
                "network": net_addr.to_string(),
                "last_address": last_addr.to_string(),
                "netmask": mask.to_string(),
                "prefix_len": v6.prefix_len(),
            });
            if let Some(n) = total {
                obj["total_addresses"] = json!(n.to_string());
            } else {
                obj["total_addresses"] =
                    json!(format!("2^{} (too large for u128)", 128 - v6.prefix_len()));
            }
            obj.to_string()
        }
    }
}

fn invert_v4(mask: Ipv4Addr) -> Ipv4Addr {
    let bits: u32 = u32::from(mask) ^ u32::MAX;
    Ipv4Addr::from(bits)
}

// ---------------------------------------------------------------------------
// net_cidr_subnets
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SubnetsArgs {
    /// Parent CIDR (IPv4 or IPv6), e.g. `10.0.0.0/24`.
    cidr: String,
    /// New (longer) prefix length for the subnets, e.g. `26` to split a /24
    /// into four /26s. Must be >= the parent prefix and <= 32 (v4) / 128 (v6).
    new_prefix: u8,
    /// Cap on the number of subnets returned (a /16 → /32 would emit 65 536
    /// rows). Defaults to 256.
    #[serde(default)]
    limit: Option<usize>,
}

pub struct NetCidrSubnets;
impl Skill for NetCidrSubnets {
    fn name(&self) -> &'static str {
        "net_cidr_subnets"
    }
    fn description(&self) -> &'static str {
        "Split a CIDR into equal-sized subnets at a longer prefix length. For a /24 with \
         `new_prefix=26` you get four /26 subnets. Each row is the subnet CIDR + its network \
         and (for v4) broadcast addresses. The `limit` arg (default 256) bounds the output."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SubnetsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<SubnetsArgs>()?;
            let parent = parse_cidr(&args.cidr)?;
            let limit = args.limit.unwrap_or(256);
            if args.new_prefix < parent.prefix_len() {
                return Err(invalid(format!(
                    "new_prefix ({}) must be >= the parent prefix ({})",
                    args.new_prefix,
                    parent.prefix_len()
                )));
            }
            let max_for_family = match parent {
                IpNet::V4(_) => 32,
                IpNet::V6(_) => 128,
            };
            if args.new_prefix > max_for_family {
                return Err(invalid(format!(
                    "new_prefix ({}) exceeds the maximum for this address family ({max_for_family})",
                    args.new_prefix
                )));
            }
            let iter: Box<dyn Iterator<Item = IpNet>> = match parent {
                IpNet::V4(v4) => Box::new(
                    v4.subnets(args.new_prefix)
                        .map_err(|e| invalid(e.to_string()))?
                        .map(IpNet::V4),
                ),
                IpNet::V6(v6) => Box::new(
                    v6.subnets(args.new_prefix)
                        .map_err(|e| invalid(e.to_string()))?
                        .map(IpNet::V6),
                ),
            };
            let mut rows = Vec::new();
            let mut truncated = false;
            for (i, sub) in iter.enumerate() {
                if i >= limit {
                    truncated = true;
                    break;
                }
                rows.push(match sub {
                    IpNet::V4(v4) => json!({
                        "cidr": v4.to_string(),
                        "network": v4.network().to_string(),
                        "broadcast": v4.broadcast().to_string(),
                    }),
                    IpNet::V6(v6) => json!({
                        "cidr": v6.to_string(),
                        "network": v6.network().to_string(),
                        "last_address": v6.broadcast().to_string(),
                    }),
                });
            }
            Ok(text_result(
                json!({
                    "parent": parent.to_string(),
                    "new_prefix": args.new_prefix,
                    "count": rows.len(),
                    "truncated": truncated,
                    "subnets": rows,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Split a /24 into /26s (four subnets)",
                args: r#"{"cidr": "192.168.1.0/24", "new_prefix": 26}"#,
                note: Some("Returns 4 subnets: .0/26, .64/26, .128/26, .192/26."),
            },
            SkillExample {
                title: "Borrow one bit (split a /24 into two /25s)",
                args: r#"{"cidr": "10.0.0.0/24", "new_prefix": 25}"#,
                note: None,
            },
            SkillExample {
                title: "Cap the output for a large split",
                args: r#"{"cidr": "10.0.0.0/16", "new_prefix": 24, "limit": 4}"#,
                note: Some("Would yield 256 subnets; `limit=4` returns the first 4 and sets truncated=true."),
            },
            SkillExample {
                title: "IPv6 split",
                args: r#"{"cidr": "2001:db8::/32", "new_prefix": 48}"#,
                note: Some("Would yield 65 536 /48s; default `limit=256` truncates."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Subdivide an allocation into per-team / per-environment ranges of equal size.",
            "Plan IP-allocation tables before configuring VLANs or VPC subnets.",
            "Compute the first N subnets of a larger block without expanding the whole range.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Length {
                field: "cidr",
                min: Some(1),
                max: None,
            },
            Rule::Range {
                field: "new_prefix",
                min: Some(0.0),
                max: Some(128.0),
            },
        ]
    }
}

// ---------------------------------------------------------------------------
// net_ip_in_cidr
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IpInCidrArgs {
    /// IPv4 or IPv6 address to test.
    ip: String,
    /// CIDR block to test membership in.
    cidr: String,
}

pub struct NetIpInCidr;
impl Skill for NetIpInCidr {
    fn name(&self) -> &'static str {
        "net_ip_in_cidr"
    }
    fn description(&self) -> &'static str {
        "Test whether an IP address (v4 or v6) falls inside a CIDR block. Returns a JSON object \
         with `contains` (bool), the canonical network the CIDR resolves to, and the position of \
         the IP within the block (0-indexed) when contained. Address families must match."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IpInCidrArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<IpInCidrArgs>()?;
            let ip: IpAddr = args
                .ip
                .trim()
                .parse()
                .map_err(|e| invalid(format!("could not parse ip `{}`: {e}", args.ip)))?;
            let net = parse_cidr(&args.cidr)?;
            let contains = net.contains(&ip);
            let mut obj = json!({
                "ip": ip.to_string(),
                "cidr": net.to_string(),
                "contains": contains,
            });
            if contains {
                let position = match (ip, &net) {
                    (IpAddr::V4(v4), IpNet::V4(parent)) => {
                        Some(json!(u32::from(v4) - u32::from(parent.network())))
                    }
                    (IpAddr::V6(v6), IpNet::V6(parent)) => {
                        let pos = u128::from(v6) - u128::from(parent.network());
                        Some(json!(pos.to_string()))
                    }
                    _ => None,
                };
                if let Some(p) = position {
                    obj["position_in_block"] = p;
                }
            }
            Ok(text_result(obj.to_string()))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Inside a /24",
                args: r#"{"ip": "10.0.0.42", "cidr": "10.0.0.0/24"}"#,
                note: Some("Returns contains=true, position_in_block=42."),
            },
            SkillExample {
                title: "Outside an off-aligned block",
                args: r#"{"ip": "10.0.1.5", "cidr": "10.0.0.0/24"}"#,
                note: Some("Returns contains=false."),
            },
            SkillExample {
                title: "IPv6 membership",
                args: r#"{"ip": "2001:db8::1", "cidr": "2001:db8::/32"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Decide whether a log-line source IP belongs to a known internal range.",
            "Validate that a configured allow-list CIDR actually covers a candidate IP.",
            "Spot off-by-one prefix errors before committing a firewall rule.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[
            Rule::Length {
                field: "ip",
                min: Some(1),
                max: None,
            },
            Rule::Length {
                field: "cidr",
                min: Some(1),
                max: None,
            },
        ]
    }
}

// ---------------------------------------------------------------------------
// net_ip_classify
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IpClassifyArgs {
    /// IPv4 or IPv6 address to classify.
    ip: String,
}

pub struct NetIpClassify;
impl Skill for NetIpClassify {
    fn name(&self) -> &'static str {
        "net_ip_classify"
    }
    fn description(&self) -> &'static str {
        "Classify an IP address against the IANA Special-Purpose Address Registry: \
         public / private (RFC 1918) / loopback / link-local / multicast / CGNAT (RFC 6598) / \
         documentation (RFC 5737, RFC 3849) / unspecified / reserved / TEREDO / 6to4. Returns a \
         category plus the citing RFC. Local compute only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IpClassifyArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<IpClassifyArgs>()?;
            let ip: IpAddr = args
                .ip
                .trim()
                .parse()
                .map_err(|e| invalid(format!("could not parse ip `{}`: {e}", args.ip)))?;
            Ok(text_result(classify_ip(ip)))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Private IPv4 (RFC 1918)",
                args: r#"{"ip": "10.42.0.1"}"#,
                note: Some("Returns category=private, rfc=1918."),
            },
            SkillExample {
                title: "CGNAT range (RFC 6598)",
                args: r#"{"ip": "100.64.0.1"}"#,
                note: Some("Returns category=cgnat, rfc=6598."),
            },
            SkillExample {
                title: "Documentation prefix",
                args: r#"{"ip": "192.0.2.1"}"#,
                note: Some("Returns category=documentation, rfc=5737."),
            },
            SkillExample {
                title: "Public IPv4",
                args: r#"{"ip": "8.8.8.8"}"#,
                note: Some("Returns category=public."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Decide whether to log / persist / share an IP based on whether it's public or private.",
            "Spot accidental documentation addresses (192.0.2.0/24, 2001:db8::/32) in production logs.",
            "Recognize CGNAT (RFC 6598) hits that look private but aren't RFC 1918.",
        ]
    }
    fn validation_rules(&self) -> &'static [crate::skills::validation::Rule] {
        use crate::skills::validation::Rule;
        &[Rule::Length {
            field: "ip",
            min: Some(1),
            max: None,
        }]
    }
}

fn classify_ip(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => classify_v4(v4),
        IpAddr::V6(v6) => classify_v6(v6),
    }
}

fn classify_v4(ip: Ipv4Addr) -> String {
    let cat = if ip.is_unspecified() {
        ("unspecified", Some("1122"))
    } else if ip.is_loopback() {
        ("loopback", Some("1122"))
    } else if ip.is_private() {
        ("private", Some("1918"))
    } else if is_cgnat(ip) {
        ("cgnat", Some("6598"))
    } else if ip.is_link_local() {
        ("link_local", Some("3927"))
    } else if ip.is_multicast() {
        ("multicast", Some("1112"))
    } else if ip.is_broadcast() {
        ("broadcast", Some("919"))
    } else if is_documentation_v4(ip) {
        ("documentation", Some("5737"))
    } else if is_reserved_v4(ip) {
        ("reserved", None)
    } else {
        ("public", None)
    };
    let mut obj = json!({
        "ip": ip.to_string(),
        "family": "IPv4",
        "category": cat.0,
    });
    if let Some(rfc) = cat.1 {
        obj["rfc"] = json!(rfc);
    }
    obj.to_string()
}

fn classify_v6(ip: Ipv6Addr) -> String {
    let cat = if ip.is_unspecified() {
        ("unspecified", Some("4291"))
    } else if ip.is_loopback() {
        ("loopback", Some("4291"))
    } else if is_link_local_v6(ip) {
        ("link_local", Some("4291"))
    } else if is_unique_local_v6(ip) {
        ("unique_local", Some("4193"))
    } else if ip.is_multicast() {
        ("multicast", Some("4291"))
    } else if is_documentation_v6(ip) {
        ("documentation", Some("3849"))
    } else if is_teredo_v6(ip) {
        ("teredo", Some("4380"))
    } else if is_6to4_v6(ip) {
        ("6to4", Some("3056"))
    } else {
        ("public", None)
    };
    let mut obj = json!({
        "ip": ip.to_string(),
        "family": "IPv6",
        "category": cat.0,
    });
    if let Some(rfc) = cat.1 {
        obj["rfc"] = json!(rfc);
    }
    obj.to_string()
}

fn is_cgnat(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (64..=127).contains(&o[1])
}

fn is_documentation_v4(ip: Ipv4Addr) -> bool {
    // RFC 5737: 192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24
    let o = ip.octets();
    (o[0] == 192 && o[1] == 0 && o[2] == 2)
        || (o[0] == 198 && o[1] == 51 && o[2] == 100)
        || (o[0] == 203 && o[1] == 0 && o[2] == 113)
}

fn is_reserved_v4(ip: Ipv4Addr) -> bool {
    // 240.0.0.0/4 (class E), excluding the 255.255.255.255 broadcast which is_broadcast() catches.
    let o = ip.octets();
    o[0] >= 240 && o[0] <= 254
}

fn is_link_local_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    (seg[0] & 0xffc0) == 0xfe80
}

fn is_unique_local_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    (seg[0] & 0xfe00) == 0xfc00
}

fn is_documentation_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    // RFC 3849: 2001:db8::/32
    seg[0] == 0x2001 && seg[1] == 0x0db8
}

fn is_teredo_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    // RFC 4380: 2001::/32
    seg[0] == 0x2001 && seg[1] == 0x0000
}

fn is_6to4_v6(ip: Ipv6Addr) -> bool {
    // RFC 3056: 2002::/16
    ip.segments()[0] == 0x2002
}

// ---------------------------------------------------------------------------
// net_cidr_summarize
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SummarizeArgs {
    /// List of CIDR blocks (mixed v4/v6 OK) to coalesce into the minimal
    /// covering set. Duplicates and overlapping ranges merge automatically.
    cidrs: Vec<String>,
}

pub struct NetCidrSummarize;
impl Skill for NetCidrSummarize {
    fn name(&self) -> &'static str {
        "net_cidr_summarize"
    }
    fn description(&self) -> &'static str {
        "Aggregate a list of CIDR blocks into the minimal covering set. Mixed IPv4 and IPv6 \
         inputs are kept separate; each family is summarized with `ipnet::IpNet::aggregate`. \
         Useful for compacting firewall rule lists, BGP advertisements, or RPKI prefix sets."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SummarizeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<SummarizeArgs>()?;
            let mut v4: Vec<Ipv4Net> = Vec::new();
            let mut v6: Vec<Ipv6Net> = Vec::new();
            for raw in &args.cidrs {
                let net = parse_cidr(raw)?;
                match net {
                    IpNet::V4(n) => v4.push(n),
                    IpNet::V6(n) => v6.push(n),
                }
            }
            let v4_out = Ipv4Net::aggregate(&v4);
            let v6_out = Ipv6Net::aggregate(&v6);
            let mut all: Vec<String> = v4_out.iter().map(|n| n.to_string()).collect();
            all.extend(v6_out.iter().map(|n| n.to_string()));
            Ok(text_result(
                json!({
                    "input_count": args.cidrs.len(),
                    "output_count": all.len(),
                    "summarized": all,
                })
                .to_string(),
            ))
        })
    }
    fn examples(&self) -> &'static [SkillExample] {
        &[
            SkillExample {
                title: "Adjacent /25s merge to a /24",
                args: r#"{"cidrs": ["10.0.0.0/25", "10.0.0.128/25"]}"#,
                note: Some("Output is one block: 10.0.0.0/24."),
            },
            SkillExample {
                title: "Non-adjacent blocks stay separate",
                args: r#"{"cidrs": ["10.0.0.0/24", "10.0.2.0/24"]}"#,
                note: Some("Cannot merge — output preserves both."),
            },
            SkillExample {
                title: "Overlapping ranges deduplicate",
                args: r#"{"cidrs": ["10.0.0.0/24", "10.0.0.0/26", "10.0.0.128/25"]}"#,
                note: Some("Output collapses to 10.0.0.0/24."),
            },
            SkillExample {
                title: "Mixed v4 + v6",
                args: r#"{"cidrs": ["10.0.0.0/25", "10.0.0.128/25", "2001:db8::/33", "2001:db8:8000::/33"]}"#,
                note: Some("Each family aggregates independently; output lists v4 then v6."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Compact a firewall ACL or RBAC allow-list before deploying it.",
            "Reduce a BGP route-table advertisement to the minimum set.",
            "Spot when a 'list of subnets' from an audit is actually one larger block.",
        ]
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn parse_cidr(s: &str) -> Result<IpNet, McpError> {
    let raw = s.trim();
    if raw.is_empty() {
        return Err(invalid("empty CIDR"));
    }
    if raw.contains('/') {
        raw.parse::<IpNet>()
            .map(|n| match n {
                IpNet::V4(v4) => IpNet::V4(v4.trunc()),
                IpNet::V6(v6) => IpNet::V6(v6.trunc()),
            })
            .map_err(|e| invalid(format!("could not parse CIDR `{s}`: {e}")))
    } else {
        // Treat a bare IP as a /32 (v4) or /128 (v6) host route.
        let ip: IpAddr = raw
            .parse()
            .map_err(|e| invalid(format!("could not parse `{s}` as IP or CIDR: {e}")))?;
        Ok(match ip {
            IpAddr::V4(v4) => IpNet::V4(Ipv4Net::new(v4, 32).unwrap()),
            IpAddr::V6(v6) => IpNet::V6(Ipv6Net::new(v6, 128).unwrap()),
        })
    }
}

// ---------------------------------------------------------------------------
// Family
// ---------------------------------------------------------------------------

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "network"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "CIDR / subnet math, IP classification, and CIDR-list aggregation. Pure local compute — \
         no DNS, no remote registries — for the binary arithmetic LLMs get wrong."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        crate::skills::SkillCapability::Ready
    }
    fn example_flow(&self) -> Option<&'static str> {
        Some(
            "1. `net_ip_classify { ip: \"10.42.7.83\" }` — is this address public?\n\
             2. `net_cidr_info { cidr: \"10.42.7.83/19\" }` — what network does it belong to, and how big is it?\n\
             3. `net_cidr_subnets { cidr: \"10.42.0.0/19\", new_prefix: 22 }` — plan the per-team subdivision.\n\
             4. `net_cidr_summarize { cidrs: [\"10.42.0.0/22\", \"10.42.4.0/22\"] }` — confirm the summary you'll advertise upstream.",
        )
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(NetCidrInfo),
        Box::new(NetCidrSubnets),
        Box::new(NetIpInCidr),
        Box::new(NetIpClassify),
        Box::new(NetCidrSummarize),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_info_v4_truncates_off_aligned() {
        let net = parse_cidr("10.42.7.83/19").unwrap();
        let s = cidr_info(&net);
        assert!(s.contains("\"network\":\"10.42.0.0\""), "{s}");
        assert!(s.contains("\"broadcast\":\"10.42.31.255\""), "{s}");
    }

    #[test]
    fn cidr_info_v4_24_has_254_usable() {
        let net = parse_cidr("192.168.1.0/24").unwrap();
        let s = cidr_info(&net);
        assert!(s.contains("\"usable_hosts\":254"), "{s}");
        assert!(s.contains("\"netmask\":\"255.255.255.0\""), "{s}");
        assert!(s.contains("\"wildcard_mask\":\"0.0.0.255\""), "{s}");
    }

    #[test]
    fn cidr_subnets_splits_24_into_four_26s() {
        let parent = parse_cidr("192.168.1.0/24").unwrap();
        let IpNet::V4(v4) = parent else { panic!() };
        let subs: Vec<_> = v4.subnets(26).unwrap().collect();
        assert_eq!(subs.len(), 4);
        assert_eq!(subs[0].network().to_string(), "192.168.1.0");
        assert_eq!(subs[1].network().to_string(), "192.168.1.64");
        assert_eq!(subs[2].network().to_string(), "192.168.1.128");
        assert_eq!(subs[3].network().to_string(), "192.168.1.192");
    }

    #[test]
    fn classify_recognizes_cgnat() {
        let s = classify_v4("100.64.0.1".parse().unwrap());
        assert!(s.contains("\"category\":\"cgnat\""), "{s}");
        assert!(s.contains("\"rfc\":\"6598\""), "{s}");
    }

    #[test]
    fn classify_recognizes_documentation_v4() {
        let s = classify_v4("192.0.2.1".parse().unwrap());
        assert!(s.contains("\"category\":\"documentation\""), "{s}");
        assert!(s.contains("\"rfc\":\"5737\""), "{s}");
    }

    #[test]
    fn classify_recognizes_documentation_v6() {
        let s = classify_v6("2001:db8::1".parse().unwrap());
        assert!(s.contains("\"category\":\"documentation\""), "{s}");
    }

    #[test]
    fn summarize_merges_adjacent_25s_into_24() {
        let v4 = vec![
            "10.0.0.0/25".parse().unwrap(),
            "10.0.0.128/25".parse().unwrap(),
        ];
        let out = Ipv4Net::aggregate(&v4);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].to_string(), "10.0.0.0/24");
    }

    #[test]
    fn ip_in_cidr_position() {
        let net = parse_cidr("10.0.0.0/24").unwrap();
        let ip: IpAddr = "10.0.0.42".parse().unwrap();
        assert!(net.contains(&ip));
    }
}
