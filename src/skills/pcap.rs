//! Packet-capture file skills — parse existing `.pcap` and `.pcapng` files via
//! the pure-Rust `pcap-file` crate. Off by default (`[pcap].enabled`). Paths
//! confined to `[filesystem].roots`. Read-only: this is for analyzing existing
//! captures, not live capture (which would need libpcap and root/admin).

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::filesystem;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, text_result};

pub const TOOL_NAMES: &[&str] = &["pcap_info", "pcap_packets"];

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PathArgs {
    /// Path to a `.pcap` (or `.pcapng`) file inside `[filesystem].roots`.
    path: String,
}

pub struct PcapInfo;
impl Skill for PcapInfo {
    fn name(&self) -> &'static str {
        "pcap_info"
    }
    fn description(&self) -> &'static str {
        "Summarize a pcap file: link layer, packet count, total bytes, first/last timestamp."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PathArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PathArgs>()?;
            let p = filesystem::resolve(&server.fs, &args.path)?;
            let f = std::fs::File::open(&p)
                .map_err(|e| internal(anyhow::anyhow!("open {}: {e}", p.display())))?;
            let mut reader = pcap_file::pcap::PcapReader::new(f)
                .map_err(|e| internal(anyhow::anyhow!("not a pcap file: {e}")))?;
            let link = format!("{:?}", reader.header().datalink);
            let mut count = 0u64;
            let mut bytes = 0u64;
            let mut first_ts: Option<(u32, u32)> = None;
            let mut last_ts: Option<(u32, u32)> = None;
            while let Some(pkt) = reader.next_packet() {
                let pkt = match pkt {
                    Ok(p) => p,
                    Err(e) => {
                        return Ok(text_result(format!(
                            "{}: parsed {count} packets then error: {e}",
                            p.display()
                        )));
                    }
                };
                count += 1;
                bytes += pkt.orig_len as u64;
                let ts = (pkt.timestamp.as_secs() as u32, pkt.timestamp.subsec_micros());
                if first_ts.is_none() {
                    first_ts = Some(ts);
                }
                last_ts = Some(ts);
            }
            let span = match (first_ts, last_ts) {
                (Some((s1, u1)), Some((s2, u2))) => {
                    let a = s1 as f64 + u1 as f64 / 1e6;
                    let b = s2 as f64 + u2 as f64 / 1e6;
                    format!(" · span {:.3}s", b - a)
                }
                _ => String::new(),
            };
            Ok(text_result(format!(
                "{}\n  link layer: {link}\n  packets: {count}\n  bytes (on-wire): {bytes}{span}",
                p.display(),
            )))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PacketsArgs {
    path: String,
    /// Skip this many packets at the start (default 0).
    #[serde(default)]
    offset: Option<u32>,
    /// Max packets to summarize (default 50, capped at 1000).
    #[serde(default)]
    max: Option<u32>,
}

pub struct PcapPackets;
impl Skill for PcapPackets {
    fn name(&self) -> &'static str {
        "pcap_packets"
    }
    fn description(&self) -> &'static str {
        "List a window of packets from a pcap with their timestamp, captured/on-wire length, \
        and a hex preview of the first 32 bytes. Use `offset` + `max` to page through."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PacketsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PacketsArgs>()?;
            let p = filesystem::resolve(&server.fs, &args.path)?;
            let f = std::fs::File::open(&p)
                .map_err(|e| internal(anyhow::anyhow!("open {}: {e}", p.display())))?;
            let mut reader = pcap_file::pcap::PcapReader::new(f)
                .map_err(|e| internal(anyhow::anyhow!("not a pcap file: {e}")))?;
            let offset = args.offset.unwrap_or(0) as usize;
            let max = args.max.unwrap_or(50).clamp(1, 1000) as usize;
            let link = format!("{:?}", reader.header().datalink);
            let mut idx = 0usize;
            let mut shown = 0usize;
            let mut out = format!("{} (link {link}):\n", p.display());
            while let Some(pkt) = reader.next_packet() {
                let pkt = match pkt {
                    Ok(p) => p,
                    Err(_) => break,
                };
                if idx >= offset && shown < max {
                    let preview_n = pkt.data.len().min(32);
                    let preview: String = pkt.data[..preview_n]
                        .iter()
                        .map(|b| format!("{:02x}", b))
                        .collect::<Vec<_>>()
                        .join(" ");
                    let ts = pkt.timestamp.as_secs_f64();
                    out.push_str(&format!(
                        "  [{:>5}] t={:.6}  len={}/{}  {preview}{}\n",
                        idx,
                        ts,
                        pkt.data.len(),
                        pkt.orig_len,
                        if pkt.data.len() > preview_n {
                            " …"
                        } else {
                            ""
                        }
                    ));
                    shown += 1;
                }
                idx += 1;
                if shown >= max {
                    break;
                }
            }
            out.push_str(&format!("\nshown {shown}, scanned {idx}"));
            Ok(text_result(out))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(PcapInfo), Box::new(PcapPackets)]
}
