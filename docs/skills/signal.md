# Signal processing — `signal_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/signal.rs`](../../src/skills/signal.rs) |
| **Tools** | `signal_fft`, `signal_dominant_frequencies`, `signal_rms`, `signal_window` |
| **Network** | none — local compute |
| **Default** | **off** — gated by `[signal]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[signal].enabled` via `LODESTONE_SIGNAL_ENABLED`. Defaults in [`src/config.rs`](../../src/config.rs). |
| **Dep** | `rustfft` (runtime SIMD via AVX/NEON auto-detect) |

## What it does

Pure-Rust DSP primitives — FFT, dominant-frequency detection, RMS, and window
function generation. Pairs with [`wave_*`](wave.md) for FFT-of-decoded-audio:
`wave_samples` decodes a `.wav` channel into f32 samples, `signal_fft` takes
them.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `signal_fft` | `samples`, `sample_rate_hz`, `window?` | Real-to-complex FFT → magnitude spectrum (frequency bin → magnitude). `window` defaults to `hann`. |
| `signal_dominant_frequencies` | `samples`, `sample_rate_hz`, `top?`, `window?` | Top-K peak frequencies (default `top=5`) — picks local maxima above a noise floor. |
| `signal_rms` | `samples` | Root-mean-square — overall signal level (dB-equivalent via `20*log10(rms)`). |
| `signal_window` | `n`, `kind` | Generate a window of length `n`. `kind`: `hann` / `hamming` / `blackman` / `rectangular`. |

## Example uses

- **Pure tone test** — generate a 440 Hz sine into `samples`, then
  `signal_dominant_frequencies { samples, sample_rate_hz: 48000 }` → 440 Hz
  at the top.
- **Audio FFT** — `wave_samples { path: "tone.wav", channel: 0 }` →
  `signal_fft { samples, sample_rate_hz: 44100 }`.
- **Compare windows** — `signal_window { n: 1024, kind: "blackman" }` for
  inspection / reuse.

## Notes

- **Real input only** (rustfft handles real→complex internally).
- **Runtime SIMD.** rustfft picks AVX2 / NEON / scalar at runtime — no
  compile-time feature flag.
- **Magnitude only.** Phase information is dropped from `signal_fft`'s output.

## See also

- [tools.md](../tools.md)
- [skills/wave.md](wave.md) — WAV decode to feed `signal_fft`.
- [skills/sdr.md](sdr.md) — RF spectrum scanning (uses `rtl_power`, not these).
