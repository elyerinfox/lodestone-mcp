# DSP extensions — `signal_*` (advanced)

|  |  |
| --- | --- |
| **Module** | [`src/skills/dsp_advanced.rs`](../../src/skills/dsp_advanced.rs) |
| **Tools** | `signal_spectrogram`, `signal_cross_correlation`, `signal_hilbert`, `signal_cepstrum`, `signal_ber_curve`, `signal_iq_demod` |
| **Network** | none — local compute |
| **Default** | on; gateable via `[tools]` |
| **Dep** | `rustfft` (runtime SIMD via AVX2 / NEON) |

## What it does

A second `signal_*` module layered on top of the existing
[`signal`](signal.md) family (FFT, dominant frequencies, RMS, window).
Adds STFT spectrogram, FFT cross-correlation with peak-lag, the analytic
signal via Hilbert, real cepstrum, BER curves for common modulations
over AWGN / Rayleigh, and IQ demodulation. Same input convention as
`signal` — real `f64` arrays plus `sample_rate_hz`.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `signal_spectrogram` | `samples`, `sample_rate_hz`, `window_size`, `overlap?` | STFT (Hann window). Returns the time × frequency power matrix (dB), bin frequencies, and frame times. |
| `signal_cross_correlation` | `a`, `b`, `sample_rate_hz` | FFT-based cross-correlation; returns the lag-indexed `corr` array, lag axis (samples), peak lag (samples / seconds), and peak value. |
| `signal_hilbert` | `samples`, `sample_rate_hz` | Hilbert transform → analytic signal (`re`, `im`), amplitude envelope, instantaneous phase, instantaneous frequency. |
| `signal_cepstrum` | `samples` | Real cepstrum (inverse FFT of log magnitude spectrum). |
| `signal_ber_curve` | `modulation`, `ebn0_db`, `m?`, `channel?` | Theoretical BER for `bpsk`, `qpsk`, `psk_m`, `qam_m`, `fsk_coherent`, `fsk_noncoherent` over `awgn` (default) or `rayleigh`. |
| `signal_iq_demod` | `i`, `q` | Magnitude, instantaneous phase, and instantaneous frequency from IQ. |

## Example uses

- **Pulse-doppler.** STFT a long signal with `window_size = 256`,
  `overlap = 0.75` → `chart_waterfall` for visual inspection.
- **Time-of-arrival.** `signal_cross_correlation` of two channels →
  `peak_lag_s` is the inter-channel delay.
- **Carrier extract.** Run `signal_hilbert` on a band-pass-filtered
  segment → `inst_freq` traces the carrier modulation.
- **Modulation comparison.** Sweep Eb/N0 from 0–12 dB through
  `signal_ber_curve` for BPSK vs 16-QAM AWGN — the right modulation pops
  out by margin.

## Notes

- For real-time / streaming work the model should chunk samples itself —
  these are batch operations on f64 buffers.
- The cepstrum is the **real** cepstrum (zero-phase reconstruction); the
  complex cepstrum is not exposed.

## See also

- [tools.md](../tools.md)
- [skills/signal.md](signal.md) — basic FFT / RMS / dominant frequencies.
- [skills/new_charts.md](new_charts.md) — `chart_waterfall` /
  `chart_density_map` for the spectrogram outputs.
