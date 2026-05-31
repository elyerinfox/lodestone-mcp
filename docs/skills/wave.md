# WAV file reader — `wave_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/wave.rs`](../../src/skills/wave.rs) |
| **Tools** | `wave_info`, `wave_samples` |
| **Network** | none |
| **Default** | **off** — gated by `[wave]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[wave].enabled` via `LODESTONE_WAVE_ENABLED`. Paths confined to `[filesystem].roots`. Defaults in [`src/config.rs`](../../src/config.rs). |
| **Dep** | `hound` (pure Rust) |

## What it does

Read a local `.wav` file: header summary, then raw decoded samples for one
channel. Pairs with [`signal_*`](signal.md) for FFT / RMS / spectrum work.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `wave_info` | `path` | Header summary: sample rate, bit depth, channels, format (PCM / IEEE float), duration in seconds. |
| `wave_samples` | `path`, `max_samples?`, `channel?` | Decode samples for one channel as f32 (default `max_samples=1024`, `channel=0`). |

## Example uses

- **Inspect a file** — `wave_info { path: "tone.wav" }`.
- **FFT a tone** — `wave_samples { path: "tone.wav", max_samples: 4096 }` →
  `signal_fft { samples, sample_rate_hz: 44100 }`.

## Notes

- **WAV only.** No MP3 / OGG / FLAC; use `ffmpeg_convert` to transcode first.
- **One channel per call.** Stereo files require two calls.

## See also

- [tools.md](../tools.md)
- [skills/signal.md](signal.md) — FFT, dominant frequencies, RMS, windowing.
- [skills/ffmpeg.md](ffmpeg.md) — convert other formats to WAV first.
