//! Binary-analysis skills: file-type detection, strings extraction, Shannon
//! entropy, hex dump, and ELF/PE/Mach-O metadata via the pure-Rust `object`
//! crate. Off by default (`[binary].enabled`). Paths confined to
//! `[filesystem].roots`. Read-only — useful for reverse engineering, malware
//! triage, and forensic file inspection.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::filesystem;
use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

pub const TOOL_NAMES: &[&str] = &[
    "binary_info",
    "binary_strings",
    "binary_entropy",
    "binary_hexdump",
];

fn read_file(server: &crate::Lodestone, path: &str) -> Result<(std::path::PathBuf, Vec<u8>), McpError> {
    let p = filesystem::resolve(&server.fs, path)?;
    let bytes = std::fs::read(&p)
        .map_err(|e| internal(anyhow::anyhow!("read {}: {e}", p.display())))?;
    Ok((p, bytes))
}

// ----- binary_info -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PathArgs {
    /// Path (must be within `[filesystem].roots`).
    path: String,
}

pub struct BinaryInfo;
impl Skill for BinaryInfo {
    fn name(&self) -> &'static str {
        "binary_info"
    }
    fn description(&self) -> &'static str {
        "Identify a binary file: detect format (ELF / PE / Mach-O / WASM / archive / unknown), \
        architecture, entry point, and a summary of sections (name, size, address). Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PathArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PathArgs>()?;
            let (p, bytes) = read_file(server, &args.path)?;
            use object::read::Object;
            let file = match object::File::parse(&*bytes) {
                Ok(f) => f,
                Err(e) => {
                    return Ok(text_result(format!(
                        "{}: not a recognized executable format ({e}). {} bytes.\n\
                         (Try binary_hexdump or binary_strings.)",
                        p.display(),
                        bytes.len()
                    )))
                }
            };
            let arch = format!("{:?}", file.architecture());
            let endian = if file.is_little_endian() { "little" } else { "big" };
            let format = format!("{:?}", file.format());
            let mut out = format!(
                "{}\n  format: {format}\n  architecture: {arch}\n  endianness: {endian}\n  entry: 0x{:x}\n  size: {} bytes\n",
                p.display(),
                file.entry(),
                bytes.len()
            );
            out.push_str("Sections:\n");
            let mut count = 0usize;
            for section in file.sections() {
                use object::read::ObjectSection;
                let name = section.name().unwrap_or("?");
                out.push_str(&format!(
                    "  {:<24}  0x{:08x}  {} bytes\n",
                    name,
                    section.address(),
                    section.size()
                ));
                count += 1;
                if count >= 32 {
                    out.push_str("  …\n");
                    break;
                }
            }
            Ok(text_result(out))
        })
    }
}

// ----- binary_strings -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StringsArgs {
    path: String,
    /// Minimum string length to report (default 4).
    #[serde(default)]
    min_length: Option<u32>,
    /// Max strings to return (default 200, capped at 5000).
    #[serde(default)]
    max: Option<u32>,
}

pub struct BinaryStrings;
impl Skill for BinaryStrings {
    fn name(&self) -> &'static str {
        "binary_strings"
    }
    fn description(&self) -> &'static str {
        "Extract printable ASCII strings (length ≥ `min_length`, default 4) from a binary, \
        like `strings(1)`. Returns up to `max` strings with their offsets."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StringsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<StringsArgs>()?;
            let (p, bytes) = read_file(server, &args.path)?;
            let min = args.min_length.unwrap_or(4).clamp(1, 1024) as usize;
            let max = args.max.unwrap_or(200).clamp(1, 5000) as usize;
            let mut out_strings: Vec<(usize, String)> = Vec::new();
            let mut start: Option<usize> = None;
            let mut cur = String::new();
            for (i, b) in bytes.iter().enumerate() {
                if (0x20..0x7f).contains(b) {
                    if start.is_none() {
                        start = Some(i);
                    }
                    cur.push(*b as char);
                } else {
                    if cur.chars().count() >= min {
                        out_strings.push((start.unwrap(), std::mem::take(&mut cur)));
                        if out_strings.len() >= max {
                            break;
                        }
                    } else {
                        cur.clear();
                    }
                    start = None;
                }
            }
            if cur.chars().count() >= min && out_strings.len() < max {
                out_strings.push((start.unwrap(), cur));
            }
            let mut out = format!(
                "{}: {} string(s) extracted (min len {}, cap {})\n",
                p.display(),
                out_strings.len(),
                min,
                max
            );
            for (off, s) in &out_strings {
                out.push_str(&format!("  0x{off:08x}  {s}\n"));
            }
            Ok(text_result(out))
        })
    }
}

// ----- binary_entropy -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct EntropyArgs {
    path: String,
    /// Block size in bytes (default 4096, capped at 1MB). Entropy is reported per block.
    #[serde(default)]
    block_size: Option<u32>,
    /// Max blocks to report (default 32, capped at 1024).
    #[serde(default)]
    max_blocks: Option<u32>,
}

pub struct BinaryEntropy;
impl Skill for BinaryEntropy {
    fn name(&self) -> &'static str {
        "binary_entropy"
    }
    fn description(&self) -> &'static str {
        "Shannon entropy per fixed-size block of a binary (0 = highly structured, 8 = nearly \
        random). Useful to spot packed / encrypted regions in malware or compressed payloads."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<EntropyArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<EntropyArgs>()?;
            let (p, bytes) = read_file(server, &args.path)?;
            let block = args.block_size.unwrap_or(4096).clamp(64, 1_048_576) as usize;
            let max_blocks = args.max_blocks.unwrap_or(32).clamp(1, 1024) as usize;
            let total_blocks = bytes.len().div_ceil(block);
            let mut out = format!(
                "{}: {} bytes, {} block(s) of {} bytes\n",
                p.display(),
                bytes.len(),
                total_blocks,
                block
            );
            out.push_str("  block   offset       entropy (bits/byte)\n");
            let mut shown_count = 0usize;
            for (i, chunk) in bytes.chunks(block).enumerate().take(max_blocks) {
                let h = shannon_entropy(chunk);
                let bar = "█".repeat(((h / 8.0) * 20.0) as usize);
                out.push_str(&format!(
                    "  {:<6}  0x{:08x}   {:.3}  {bar}\n",
                    i,
                    i * block,
                    h
                ));
                shown_count = i + 1;
            }
            if total_blocks > shown_count {
                out.push_str(&format!(
                    "  … {} more blocks truncated\n",
                    total_blocks - shown_count
                ));
            }
            Ok(text_result(out))
        })
    }
}

fn shannon_entropy(bytes: &[u8]) -> f64 {
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0u32; 256];
    for &b in bytes {
        counts[b as usize] += 1;
    }
    let n = bytes.len() as f64;
    let mut h = 0.0;
    for &c in &counts {
        if c == 0 {
            continue;
        }
        let p = c as f64 / n;
        h -= p * p.log2();
    }
    h
}

// ----- binary_hexdump -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HexdumpArgs {
    path: String,
    /// Start offset (default 0).
    #[serde(default)]
    offset: Option<u64>,
    /// Length to dump (default 256, capped at 8192).
    #[serde(default)]
    length: Option<u32>,
}

pub struct BinaryHexdump;
impl Skill for BinaryHexdump {
    fn name(&self) -> &'static str {
        "binary_hexdump"
    }
    fn description(&self) -> &'static str {
        "Classic 16-byte-per-row hex dump of a region of a file: offset, hex bytes, ASCII \
        gutter. Pass `offset` and `length` to dump just a window."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HexdumpArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<HexdumpArgs>()?;
            let (p, bytes) = read_file(server, &args.path)?;
            let off = args.offset.unwrap_or(0) as usize;
            let len = args.length.unwrap_or(256).clamp(1, 8192) as usize;
            if off >= bytes.len() {
                return Err(invalid(format!(
                    "offset {off} is past end of file ({} bytes)",
                    bytes.len()
                )));
            }
            let end = (off + len).min(bytes.len());
            let region = &bytes[off..end];
            let mut out = format!(
                "{}: hexdump {} bytes from 0x{:x} (file size {})\n",
                p.display(),
                region.len(),
                off,
                bytes.len()
            );
            for (i, chunk) in region.chunks(16).enumerate() {
                let addr = off + i * 16;
                let mut hex = String::new();
                let mut ascii = String::new();
                for (j, b) in chunk.iter().enumerate() {
                    hex.push_str(&format!("{:02x} ", b));
                    if j == 7 {
                        hex.push(' ');
                    }
                    ascii.push(if (0x20..0x7f).contains(b) {
                        *b as char
                    } else {
                        '.'
                    });
                }
                // pad hex column when the last row is short
                while hex.len() < 16 * 3 + 1 {
                    hex.push(' ');
                }
                out.push_str(&format!("{:08x}  {hex} |{ascii}|\n", addr));
            }
            Ok(text_result(out))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(BinaryInfo),
        Box::new(BinaryStrings),
        Box::new(BinaryEntropy),
        Box::new(BinaryHexdump),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entropy_uniform_is_high_constant_is_zero() {
        let zeros = vec![0u8; 1024];
        assert!(shannon_entropy(&zeros) < 0.001);
        let uniform: Vec<u8> = (0..1024).map(|i| (i & 0xff) as u8).collect();
        assert!(shannon_entropy(&uniform) > 7.5);
    }
}
