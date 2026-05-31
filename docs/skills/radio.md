# Radio link math — `radio_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/radio.rs`](../../src/skills/radio.rs) |
| **Tools** | `radio_fspl`, `radio_link_budget`, `radio_antenna` |
| **Network** | none — local compute |
| **Default** | **off** — gated by `[radio]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[radio].enabled` via `LODESTONE_RADIO_ENABLED`. Defaults in [`src/config.rs`](../../src/config.rs). |

## What it does

RF link math, the calculations every amateur / SWL / WISP designer reaches
for. Pure formulas — no propagation modeling beyond free-space.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `radio_fspl` | `frequency_hz`, `distance_m` | Free-space path loss in dB. |
| `radio_link_budget` | `tx_power_dbm`, `tx_gain_dbi`, `rx_gain_dbi`, `frequency_hz`, `distance_m`, `cable_loss_db?`, `other_losses_db?` | Received power and link margin against a typical noise floor. |
| `radio_antenna` | `frequency_hz`, `gain_dbi?`, `effective_aperture_m2?` | Convert antenna gain ↔ effective aperture (give one, get the other). |

## Example uses

- **2.4 GHz link, 1 km** —
  `radio_fspl { frequency_hz: 2.4e9, distance_m: 1000 }`.
- **Plan a UHF point-to-point** —
  `radio_link_budget { tx_power_dbm: 30, tx_gain_dbi: 12, rx_gain_dbi: 12, frequency_hz: 450e6, distance_m: 8000 }`.
- **What aperture is a 24 dBi dish at 10 GHz?** —
  `radio_antenna { frequency_hz: 10e9, gain_dbi: 24 }`.

## Notes

- **Free-space only.** No terrain, no Fresnel zone clearance, no fading
  margin beyond what you supply. For NLOS / urban, plug a heuristic margin
  into `other_losses_db`.
- **For US band rules**, see [`fcc`](fcc.md) (`fcc_amateur_bands`,
  `fcc_radio_service`).

## See also

- [tools.md](../tools.md)
- [skills/fcc.md](fcc.md) — band plans + power caps.
- [skills/sdr.md](sdr.md) — actually receive.
- [skills/signal.md](signal.md) — DSP on captured I/Q.
