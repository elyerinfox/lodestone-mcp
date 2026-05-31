# x86 / x64 disassembler — `disasm_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/disasm.rs`](../../src/skills/disasm.rs) |
| **Tools** | `disasm_x86_hex`, `disasm_x86_file` |
| **Network** | none |
| **Default** | **off** — gated by `[disasm]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[disasm].enabled` via `LODESTONE_DISASM_ENABLED`. Paths confined to `[filesystem].roots`. Defaults in [`src/config.rs`](../../src/config.rs). |
| **Dep** | `iced-x86` (`fast_fmt`, pure Rust) |

## What it does

Disassemble x86 / x64 instructions — from a hex string (great for a quick
look at bytes you already have) or from a region of a local binary. NASM
flavor output, the standard form for reading.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `disasm_x86_hex` | `hex`, `bits?`, `base?`, `max?` | Disassemble inline hex (e.g. `"48 89 e5 5d c3"`). `bits`: `16` / `32` / `64` (default 64). `base`: starting RIP (default 0). |
| `disasm_x86_file` | `path`, `offset?`, `length?`, `bits?` | Disassemble a region of a local file (default first 512 bytes). |

## Example uses

- **What does this byte sequence do?** —
  `disasm_x86_hex { hex: "48 89 e5 5d c3", bits: 64 }` →
  `mov rbp, rsp` / `pop rbp` / `ret`.
- **Walk an entry point** — `binary_info` for the entry, then
  `disasm_x86_file { path, offset: <entry>, length: 256 }`.

## Notes

- **x86 / x64 only.** No ARM / RISC-V (yet).
- **NASM flavor.** Intel syntax with NASM conventions.
- **Read-only.** No assembly / patching.

## See also

- [tools.md](../tools.md)
- [skills/binary.md](binary.md) — find the entry point / sections first.
