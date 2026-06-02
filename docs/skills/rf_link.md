# RF link engineering — `rf_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/rf_link.rs`](../../src/skills/rf_link.rs) |
| **Tools** | `rf_two_ray_path_loss`, `rf_hata_path_loss`, `rf_cost231_path_loss`, `rf_egli_path_loss`, `rf_itu_p676_absorption`, `rf_itu_p838_rain`, `rf_doppler_shift`, `rf_polarization_loss`, `rf_fresnel_zone_radius`, `rf_knife_edge_diffraction`, `rf_friis_with_noise` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |

## What it does

Path-loss and link-budget primitives for ground-mobile, point-to-point, and
satellite links. The previously-available `radio_*` family covered Friis
free-space and a generic link budget; this module adds the empirical /
semi-empirical models the radio bands actually live in, plus rain /
atmospheric absorption, Fresnel-zone geometry, knife-edge diffraction, and a
Friis variant that folds in receiver bandwidth and noise.

## Tools

### Path loss

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `rf_two_ray_path_loss` | `frequency_hz`, `tx_height_m`, `rx_height_m`, `distance_m` | Plane-earth two-ray model: PL ≈ 40·log d − 20·log(hₜ·hᵣ). |
| `rf_hata_path_loss` | `frequency_mhz`, `bs_height_m`, `mobile_height_m`, `distance_km`, `environment` | Okumura-Hata (150–1500 MHz). `environment`: `urban_large`, `urban_small`, `suburban`, `open`. |
| `rf_cost231_path_loss` | `frequency_mhz`, `bs_height_m`, `mobile_height_m`, `distance_km`, `environment` | COST-231-Hata extension (1500–2000 MHz). `environment`: `medium_small_cities` (default add 0 dB) or `metro_large` (+3 dB). |
| `rf_egli_path_loss` | `frequency_mhz`, `tx_height_m`, `rx_height_m`, `distance_km` | Egli VHF/UHF irregular-terrain model. |

### Atmospheric / weather attenuation

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `rf_itu_p676_absorption` | `frequency_ghz`, `pressure_hpa?`, `temp_c?`, `water_vapor_g_m3?` | ITU-R P.676 oxygen + water-vapor absorption (simplified line-by-line). |
| `rf_itu_p838_rain` | `frequency_ghz`, `rain_rate_mm_h`, `polarization` | ITU-R P.838 specific attenuation γ = k·R^α. `polarization`: `horizontal`, `vertical`, `circular`. |

### Link geometry & physics

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `rf_doppler_shift` | `frequency_hz`, `velocity_m_s` | Δf = v·f/c (+ closing). |
| `rf_polarization_loss` | `tx`, `rx`, `tx_angle_deg?`, `rx_angle_deg?` | Mismatch loss (dB). Polarizations: `linear_h`, `linear_v`, `linear_at_deg`, `rhcp`, `lhcp`. |
| `rf_fresnel_zone_radius` | `frequency_hz`, `distance_m`, `distance_to_obstruction_m`, `n?` | F_n = √(n·λ·d₁·d₂/d). |
| `rf_knife_edge_diffraction` | `frequency_hz`, `d1_m`, `d2_m`, `h_m` | Knife-edge diffraction loss (Lee's J(v) approximation). |

### Link budget

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `rf_friis_with_noise` | `frequency_hz`, `distance_m`, `tx_power_dbm`, `tx_gain_dbi`, `rx_gain_dbi`, `bandwidth_hz`, `extra_loss_db?`, `noise_figure_db?`, `required_snr_db?` | Friis + kTBF system noise floor; returns received power, noise floor, SNR, link margin. |

## Example uses

- **Site survey.** Pick `rf_hata_path_loss` for a 900 MHz cell-tower
  link; compare to `rf_egli_path_loss` for the hill-and-valley case.
- **Microwave link.** At 12 GHz across 30 km, sum
  `rf_itu_p676_absorption` + `rf_itu_p838_rain` (24 mm/h, vertical) into
  `extra_loss_db` for `rf_friis_with_noise`.
- **Mountain path.** Use `rf_fresnel_zone_radius` to check the first
  Fresnel-zone clearance, then `rf_knife_edge_diffraction` for the
  remaining obstacle loss.

## Notes

- All path-loss tools return one number (loss in dB). They DON'T add
  free-space loss back in — they ARE the model output.
- ITU-R P.676 in this module is a simplified line-by-line — accurate to
  a few dB over the standard atmosphere bands; for design margins use a
  ground-truth tool.

## See also

- [tools.md](../tools.md)
- [skills/radio.md](radio.md) — basic Friis + simple link budget +
  antenna gain ↔ aperture.
- [skills/radar.md](radar.md) — radar equation family.
- [skills/atmospheric.md](atmospheric.md) — humidity → water-vapor input.
