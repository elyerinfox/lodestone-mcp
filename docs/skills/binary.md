# Binary analysis — `binary_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/binary.rs`](../../src/skills/binary.rs) |
| **Tools** | `binary_info`, `binary_strings`, `binary_entropy`, `binary_hexdump` |
| **Network** | none — pure-Rust binary parsing |
| **Default** | **off** — gated by `[binary]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[binary].enabled` via `LODESTONE_BINARY_ENABLED`. Paths confined to `[filesystem].roots`. Defaults in [`src/config.rs`](../../src/config.rs). |
| **Dep** | `object` crate (pure Rust, multi-format) |

## What it does

Read-only forensic inspection of binaries — executable-format detection,
printable-string extraction, Shannon entropy (for spotting packed / encrypted
regions), and a hexdump. Useful for reverse engineering, malware triage, and
forensic file inspection.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `binary_info` | `path` | Identify the format (ELF / PE / Mach-O / WASM / archive / unknown), arch, entry point, sections. |
| `binary_strings` | `path`, `min_len?`, `max?` | Extract printable strings (default `min_len=6`). |
| `binary_entropy` | `path`, `block_size?` | Per-block Shannon entropy (default 4 KiB). High-entropy blocks suggest packing / encryption / compression. |
| `binary_hexdump` | `path`, `offset?`, `length?` | Classic hexdump of a byte range (default 256 bytes from offset 0). |

## Example uses

- **What is this?** — `binary_info { path: "suspicious.bin" }`.
- **Find URLs / strings** — `binary_strings { path: "dropper.exe", min_len: 12 }`.
- **Spot a packed section** — `binary_entropy { path: "elf-bin" }`; look for
  blocks at ~7.9 bits/byte (uniformly random).
- **Inspect a header** — `binary_hexdump { path: "elf-bin", length: 64 }`.

## Notes

- **Read-only.** No execution, no patching.
- **Format detection** via the `object` crate's magic-byte sniffing.

## See also

- [tools.md](../tools.md)
- [skills/disasm.md](disasm.md) — actually disassemble the code.
- [skills/pcap.md](pcap.md) — paired with packet captures during triage.
