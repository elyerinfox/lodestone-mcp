# Radar equation family — `radar_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/radar.rs`](../../src/skills/radar.rs) |
| **Tools** | `radar_monostatic`, `radar_bistatic`, `radar_integration_gain`, `radar_pulse_compression_gain`, `radar_cfar_threshold`, `radar_clutter_threshold`, `radar_doppler_shift` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |

## What it does

Closed-form radar range equations + integration / pulse-compression /
CFAR / clutter helpers. Antenna gains are passed as **linear ratios**
(not dBi) so the equations stay clean; convert with `10^(dBi/10)` before
the call.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `radar_monostatic` | `pt_w`, `gain`, `wavelength_m`, `rcs_m2`, `range_m`, `bandwidth_hz`, `noise_temp_k?`, `losses_db?` | Pr = Pt·G²·λ²·σ / ((4π)³·R⁴·L); returns received power + SNR. |
| `radar_bistatic` | `pt_w`, `gt`, `gr`, `wavelength_m`, `sigma_b_m2`, `rt_m`, `rr_m`, `bandwidth_hz`, `noise_temp_k?`, `losses_db?` | Bistatic variant (Tx/Rx separated). |
| `radar_integration_gain` | `n`, `method` | Coherent: 10·log₁₀(N). Non-coherent: with Marcum loss. |
| `radar_pulse_compression_gain` | `pulse_width_s`, `bandwidth_hz` | Time-bandwidth product τ·B (dB). |
| `radar_cfar_threshold` | `n_cells`, `pfa`, `method`, `k?` | CA-CFAR threshold multiplier; `method`=`os` requires order-statistics rank `k`. |
| `radar_clutter_threshold` | `distribution`, `pfa`, `shape?` | Rayleigh / Weibull (`shape`=k) / K-distribution (`shape`=ν) detection threshold for a given Pfa. |
| `radar_doppler_shift` | `frequency_hz`, `radial_velocity_m_s` | 2·v·f/c radar Doppler. |

## Example uses

- **Detection range.** Set the target SNR floor (say 13 dB), iterate on
  `range_m` until `radar_monostatic` returns it — that's your range.
- **Coherent vs non-coherent.** Same `n`, both methods through
  `radar_integration_gain` — coherent gain is typically 1–3 dB better at
  high N.
- **False-alarm sizing.** `radar_cfar_threshold { n_cells: 16, pfa: 1e-6,
  method: "ca" }` → the multiplier you multiply your noise estimate by
  to set the detection threshold.

## Notes

- "Antenna gain" arguments are linear ratios (e.g. 1000), not dB.
- The non-coherent integration assumes square-law detection (Marcum loss
  on top of N).

## See also

- [tools.md](../tools.md)
- [skills/rf_link.md](rf_link.md) — atmospheric / rain / polarization
  losses to fold into `losses_db`.
- [skills/tracking.md](tracking.md) — pair `radar_cfar_threshold`
  detections with `track_kalman_step`.
