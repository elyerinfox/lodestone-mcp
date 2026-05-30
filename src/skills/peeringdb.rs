//! PeeringDB skills — keyless public REST API at `peeringdb.com/api/`.
//! Look up networks (ASNs), internet exchanges (IXs), facilities (colos), and
//! organizations. Useful for interconnection planning and figuring out where
//! a given network peers.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

fn url_enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

async fn pdb_get(server: &crate::Lodestone, url: &str) -> Result<Value, McpError> {
    let r = server
        .http
        .get(url)
        .header("Accept", "application/json")
        .send()
        .await
        .and_then(|x| x.error_for_status())
        .map_err(|e| internal(anyhow::anyhow!("peeringdb: {e}")))?;
    r.json()
        .await
        .map_err(|e| internal(anyhow::anyhow!("peeringdb parse: {e}")))
}

// ----- peeringdb_network -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NetArgs {
    /// AS number to look up (preferred) — e.g. 13335 (Cloudflare). Either `asn` or `name` must be set.
    #[serde(default)]
    asn: Option<u32>,
    /// Name substring (e.g. "cloudflare"). Used if `asn` is omitted.
    #[serde(default)]
    name: Option<String>,
    /// Max results (default 10, capped at 50).
    #[serde(default)]
    max: Option<u32>,
}

pub struct PeeringDbNetwork;
impl Skill for PeeringDbNetwork {
    fn name(&self) -> &'static str {
        "peeringdb_network"
    }
    fn description(&self) -> &'static str {
        "Look up networks (autonomous systems) in PeeringDB by ASN or name substring. Returns \
        AS number, name, info_type, info_traffic, info_prefixes4/6, and a list of IX presences."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NetArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<NetArgs>()?;
            if args.asn.is_none() && args.name.as_ref().is_none_or(|n| n.trim().is_empty()) {
                return Err(invalid("provide either `asn` or `name`"));
            }
            let max = args.max.unwrap_or(10).clamp(1, 50);
            let url = if let Some(asn) = args.asn {
                format!("https://www.peeringdb.com/api/net?asn={asn}&limit={max}")
            } else {
                let q = args.name.unwrap();
                format!(
                    "https://www.peeringdb.com/api/net?name__contains={}&limit={max}",
                    url_enc(q.trim())
                )
            };
            let v = pdb_get(server, &url).await?;
            let empty = Vec::new();
            let data = v.get("data").and_then(|x| x.as_array()).unwrap_or(&empty);
            if data.is_empty() {
                return Ok(text_result("No networks match.".to_string()));
            }
            let mut out = format!("{} network(s):\n", data.len());
            for n in data {
                let asn = n.get("asn").and_then(|x| x.as_i64()).unwrap_or(0);
                let name = n.get("name").and_then(|x| x.as_str()).unwrap_or("");
                let typ = n.get("info_type").and_then(|x| x.as_str()).unwrap_or("");
                let traffic = n.get("info_traffic").and_then(|x| x.as_str()).unwrap_or("");
                let p4 = n
                    .get("info_prefixes4")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
                let p6 = n
                    .get("info_prefixes6")
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0);
                out.push_str(&format!(
                    "  AS{asn}  {name}\n    {typ} · {traffic} · IPv4 prefixes {p4}, IPv6 prefixes {p6}\n"
                ));
            }
            Ok(text_result(out))
        })
    }
}

// ----- peeringdb_exchange -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IxArgs {
    /// Name substring (e.g. "amsix", "linx", "decix").
    #[serde(default)]
    name: Option<String>,
    /// ISO country code filter (e.g. "US", "DE").
    #[serde(default)]
    country: Option<String>,
    /// City substring.
    #[serde(default)]
    city: Option<String>,
    /// Max results (default 10, capped at 50).
    #[serde(default)]
    max: Option<u32>,
}

pub struct PeeringDbExchange;
impl Skill for PeeringDbExchange {
    fn name(&self) -> &'static str {
        "peeringdb_exchange"
    }
    fn description(&self) -> &'static str {
        "Look up internet exchanges (IXs) in PeeringDB by name, country, and/or city. Returns \
        name, city, country, organization, and member count."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IxArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<IxArgs>()?;
            let max = args.max.unwrap_or(10).clamp(1, 50);
            let mut url = format!("https://www.peeringdb.com/api/ix?limit={max}");
            if let Some(n) = args
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                url.push_str(&format!("&name__contains={}", url_enc(n)));
            }
            if let Some(c) = args
                .country
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                url.push_str(&format!("&country={}", url_enc(&c.to_ascii_uppercase())));
            }
            if let Some(c) = args
                .city
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                url.push_str(&format!("&city__contains={}", url_enc(c)));
            }
            let v = pdb_get(server, &url).await?;
            let empty = Vec::new();
            let data = v.get("data").and_then(|x| x.as_array()).unwrap_or(&empty);
            if data.is_empty() {
                return Ok(text_result("No IXes match.".to_string()));
            }
            let mut out = format!("{} IX(es):\n", data.len());
            for ix in data {
                let id = ix.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
                let name = ix.get("name").and_then(|x| x.as_str()).unwrap_or("");
                let city = ix.get("city").and_then(|x| x.as_str()).unwrap_or("");
                let country = ix.get("country").and_then(|x| x.as_str()).unwrap_or("");
                let org = ix.get("org_name").and_then(|x| x.as_str()).unwrap_or("");
                let net_count = ix.get("net_count").and_then(|x| x.as_i64()).unwrap_or(0);
                out.push_str(&format!(
                    "  ix-{id}  {name}\n    {city}, {country} · {org} · {net_count} member(s)\n"
                ));
            }
            Ok(text_result(out))
        })
    }
}

// ----- peeringdb_facility -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FacArgs {
    /// Name substring (e.g. "equinix").
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    country: Option<String>,
    #[serde(default)]
    city: Option<String>,
    #[serde(default)]
    max: Option<u32>,
}

pub struct PeeringDbFacility;
impl Skill for PeeringDbFacility {
    fn name(&self) -> &'static str {
        "peeringdb_facility"
    }
    fn description(&self) -> &'static str {
        "Look up colocation facilities (carrier-neutral data centres) in PeeringDB by name, \
        country, and/or city. Returns name, address, operator, and tenancy counts (networks, \
        exchanges present)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FacArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<FacArgs>()?;
            let max = args.max.unwrap_or(10).clamp(1, 50);
            let mut url = format!("https://www.peeringdb.com/api/fac?limit={max}");
            if let Some(n) = args
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                url.push_str(&format!("&name__contains={}", url_enc(n)));
            }
            if let Some(c) = args
                .country
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                url.push_str(&format!("&country={}", url_enc(&c.to_ascii_uppercase())));
            }
            if let Some(c) = args
                .city
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                url.push_str(&format!("&city__contains={}", url_enc(c)));
            }
            let v = pdb_get(server, &url).await?;
            let empty = Vec::new();
            let data = v.get("data").and_then(|x| x.as_array()).unwrap_or(&empty);
            if data.is_empty() {
                return Ok(text_result("No facilities match.".to_string()));
            }
            let mut out = format!("{} facility(ies):\n", data.len());
            for f in data {
                let id = f.get("id").and_then(|x| x.as_i64()).unwrap_or(0);
                let name = f.get("name").and_then(|x| x.as_str()).unwrap_or("");
                let addr = f.get("address1").and_then(|x| x.as_str()).unwrap_or("");
                let city = f.get("city").and_then(|x| x.as_str()).unwrap_or("");
                let country = f.get("country").and_then(|x| x.as_str()).unwrap_or("");
                let org = f.get("org_name").and_then(|x| x.as_str()).unwrap_or("");
                let net_count = f.get("net_count").and_then(|x| x.as_i64()).unwrap_or(0);
                let ix_count = f.get("ix_count").and_then(|x| x.as_i64()).unwrap_or(0);
                out.push_str(&format!(
                    "  fac-{id}  {name}\n    {addr}, {city}, {country} · {org} · {net_count} net(s) · {ix_count} IX(es)\n"
                ));
            }
            Ok(text_result(out))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(PeeringDbNetwork),
        Box::new(PeeringDbExchange),
        Box::new(PeeringDbFacility),
    ]
}
