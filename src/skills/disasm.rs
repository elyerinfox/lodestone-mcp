//! x86/x64 disassembly via `iced-x86` (pure Rust). Off by default
//! (`[disasm].enabled`). Two tools: `disasm_x86_hex` decodes a hex string;
//! `disasm_x86_file` reads bytes from a file (path confined to
//! `[filesystem].roots`). Both produce NASM-flavored output.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{fs_read_bytes, schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

fn parse_hex(s: &str) -> Result<Vec<u8>, McpError> {
    // Strip `0x` / `0X` prefixes per token so inputs like `"0x90 0xCC"` parse
    // (the field's doc-comment promises this; without per-token stripping the
    // `x` is dropped while the surrounding `0` is kept, leaving `090CC` and an
    // odd-length error).
    let stripped: String = s
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .map(|tok| tok.trim_start_matches("0x").trim_start_matches("0X"))
        .collect::<Vec<_>>()
        .join("");
    let cleaned: String = stripped.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if cleaned.is_empty() {
        return Err(invalid("no hex digits in bytes_hex"));
    }
    if !cleaned.len().is_multiple_of(2) {
        return Err(invalid("bytes_hex has an odd number of hex digits"));
    }
    let mut out = Vec::with_capacity(cleaned.len() / 2);
    for i in (0..cleaned.len()).step_by(2) {
        let byte = u8::from_str_radix(&cleaned[i..i + 2], 16)
            .map_err(|e| invalid(format!("bad hex byte: {e}")))?;
        out.push(byte);
    }
    Ok(out)
}

fn decode_bytes(bytes: &[u8], bits: u32, address: u64) -> Result<String, McpError> {
    let bitness = match bits {
        16 => 16u32,
        32 => 32,
        64 => 64,
        _ => return Err(invalid("bits must be 16, 32, or 64")),
    };
    let mut decoder = iced_x86::Decoder::with_ip(bitness, bytes, address, 0);
    let mut formatter = iced_x86::NasmFormatter::new();
    let mut out = String::new();
    let mut instr = iced_x86::Instruction::default();
    let mut line = String::new();
    while decoder.can_decode() {
        decoder.decode_out(&mut instr);
        line.clear();
        use iced_x86::Formatter;
        formatter.format(&instr, &mut line);
        let start = (instr.ip() - address) as usize;
        let end = start + instr.len();
        let bytes_str: String = bytes[start..end]
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ");
        out.push_str(&format!(
            "  {:016x}  {:<24}  {line}\n",
            instr.ip(),
            bytes_str
        ));
    }
    Ok(out)
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct HexArgs {
    /// Hex bytes (whitespace, commas, `0x` prefixes are stripped).
    bytes_hex: String,
    /// Architecture bitness: 16, 32, or 64.
    bits: u32,
    /// Starting virtual address (default 0).
    #[serde(default)]
    address: Option<u64>,
}

pub struct DisasmHex;
impl Skill for DisasmHex {
    fn name(&self) -> &'static str {
        "disasm_x86_hex"
    }
    fn description(&self) -> &'static str {
        "Disassemble a hex byte string as x86/x64 (NASM syntax). `bits` is 16, 32, or 64. \
        Useful for shellcode / one-off instruction decoding."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<HexArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<HexArgs>()?;
            let bytes = parse_hex(&args.bytes_hex)?;
            let addr = args.address.unwrap_or(0);
            let body = decode_bytes(&bytes, args.bits, addr)?;
            Ok(text_result(format!(
                "x86 disassembly ({} bits, {} bytes, base 0x{:x}):\n{body}",
                args.bits,
                bytes.len(),
                addr
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "64-bit nops",
                args: r#"{"bytes_hex": "909090", "bits": 64}"#,
                note: Some("Three `nop` instructions in NASM syntax."),
            },
            SkillExample {
                title: "32-bit shellcode at a virtual base",
                args: r#"{"bytes_hex": "31c0 40", "bits": 32, "address": 4096}"#,
                note: Some("Whitespace and `0x` prefixes in bytes_hex are stripped."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Decode shellcode pasted from CTF / exploit-dev material.",
            "Spot-check a few opcodes without staging them in a file first.",
            "Validate hand-assembled bytes before patching them into a binary.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FileArgs {
    /// File to read bytes from (must be inside `[filesystem].roots`).
    path: String,
    /// Byte offset into the file.
    offset: u64,
    /// Number of bytes to disassemble (capped at 65536).
    length: u32,
    /// Architecture bitness: 16, 32, or 64.
    bits: u32,
    /// Virtual address that `offset` corresponds to (default = `offset`).
    #[serde(default)]
    address: Option<u64>,
}

pub struct DisasmFile;
impl Skill for DisasmFile {
    fn name(&self) -> &'static str {
        "disasm_x86_file"
    }
    fn description(&self) -> &'static str {
        "Disassemble a slice of bytes from a file as x86/x64 (NASM syntax). Pair with \
        binary_info to find the .text section's file offset and runtime address."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<FileArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<FileArgs>()?;
            let length = (args.length).clamp(1, 65536) as usize;
            let (p, bytes) = fs_read_bytes(server, &args.path)?;
            let off = args.offset as usize;
            if off >= bytes.len() {
                return Err(invalid(format!(
                    "offset {off} past end of file ({} bytes)",
                    bytes.len()
                )));
            }
            let end = (off + length).min(bytes.len());
            let region = &bytes[off..end];
            let addr = args.address.unwrap_or(args.offset);
            let body = decode_bytes(region, args.bits, addr)?;
            Ok(text_result(format!(
                "x86 disassembly of {} bytes from {}:0x{:x} ({} bits, base 0x{:x}):\n{body}",
                region.len(),
                p.display(),
                off,
                args.bits,
                addr
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Disassemble .text from a 64-bit ELF",
                args: r#"{"path": "samples/hello.elf", "offset": 4096, "length": 256, "bits": 64, "address": 4198400}"#,
                note: Some(
                    "Use binary_info to find the section's file offset and runtime address first.",
                ),
            },
            SkillExample {
                title: "Decode a small slice as 32-bit",
                args: r#"{"path": "samples/payload.bin", "offset": 0, "length": 64, "bits": 32}"#,
                note: Some("Omits address; instruction IPs default to `offset`."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Disassemble a known code region of a binary on disk without a debugger.",
            "Pair with binary_info to walk each section's machine code in order.",
            "Reverse-engineer a small function when you already have its file offset.",
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(DisasmHex), Box::new(DisasmFile)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_simple_x64_nop() {
        // 0x90 = nop
        let out = decode_bytes(&[0x90, 0x90, 0x90], 64, 0).unwrap();
        assert!(out.lines().count() >= 3, "{out}");
        assert!(out.contains("nop"));
    }
}
