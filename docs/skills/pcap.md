# Packet capture reader — `pcap_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/pcap.rs`](../../src/skills/pcap.rs) |
| **Tools** | `pcap_info`, `pcap_packets` |
| **Network** | none — read-only file parsing |
| **Default** | **off** — gated by `[pcap]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[pcap].enabled` via `LODESTONE_PCAP_ENABLED`. Paths confined to `[filesystem].roots`. Defaults in [`src/config.rs`](../../src/config.rs). |
| **Dep** | `pcap-file` (pure Rust — no native `libpcap`) |

## What it does

Read a `.pcap` file (the classic `tcpdump` / Wireshark format). No live capture
— this only inspects saved files. Pairs naturally with the binary / disasm
families during forensic triage.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `pcap_info` | `path` | Header summary: datalink type, snaplen, packet count. |
| `pcap_packets` | `path`, `offset?`, `max?` | Walk packet records — timestamps + lengths + first bytes (default first 20, capped at 1000). |

## Example uses

- **Quick triage** — `pcap_info { path: "capture.pcap" }`.
- **Walk packets** — `pcap_packets { path: "capture.pcap", max: 50 }`.

## Notes

- **Read-only.** No injection / replay.
- **Classic pcap only.** No PcapNG (yet).
- **No protocol decode.** The model gets raw record bytes; if you need
  TCP / HTTP / DNS parsing, feed the bytes into a downstream tool.

## See also

- [tools.md](../tools.md)
- [skills/binary.md](binary.md) — paired during forensic work.
