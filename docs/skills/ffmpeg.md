# Media conversion — `ffmpeg_probe`, `ffmpeg_convert`

|  |  |
| --- | --- |
| **Module** | [`src/skills/ffmpeg.rs`](../../src/skills/ffmpeg.rs) |
| **Tools** | `ffmpeg_probe`, `ffmpeg_convert` |
| **Network** | none (shells out to local `ffmpeg`/`ffprobe`) |
| **Default** | **off** — gated by `[ffmpeg]` |
| **Config** | `[ffmpeg]` in [`config/10-filesystem.toml`](../../config/10-filesystem.toml) |

## What it does
Probes and converts local media by shelling out to a system **FFmpeg** install.
There's no good pure-Rust transcoder, so this wraps the `ffmpeg`/`ffprobe`
binaries. Off by default; if FFmpeg isn't on `PATH`, the error says so.

- Every `input`/`output` path is **confined to `[filesystem].roots`** (the same
  `..`/symlink rules as the filesystem skill), independent of whether the
  filesystem *tools* are enabled.
- `ffmpeg_convert` writes a file, so it's routed through the confirmation
  [guard](../golden-rules.md): the first call returns a token and does nothing; call
  again with `confirm=<token>` (or `confirm` + `trust=true`).

## Tools
| Tool | Arguments | Access | Purpose |
| --- | --- | --- | --- |
| `ffmpeg_probe` | `input` | read | Container format, duration, bitrate, and per-stream codec/resolution/sample-rate (via `ffprobe`). |
| `ffmpeg_convert` | `input`, `output`, `args?`, `confirm?`, `trust?` | **write** | Convert/transcode; format inferred from `output`'s extension unless overridden by `args`. |

`args` is a **pre-split** list of extra ffmpeg flags inserted between `-i input`
and `output` — e.g. `["-vf", "scale=1280:-1", "-c:v", "libx264", "-crf", "23"]`.
They're passed to the process directly (no shell), so metacharacters are inert.

## Example uses
- **Inspect a file** — `ffmpeg_probe { input: "clip.mov" }`.
- **Re-encode to MP4** — `ffmpeg_convert { input: "clip.mov", output: "clip.mp4" }`
  → returns a confirm token; repeat with `confirm`.
- **Extract audio** — `ffmpeg_convert { input: "talk.mp4", output: "talk.mp3" }`.
- **Downscale** — add `args: ["-vf", "scale=1280:-1"]`.

## Notes
- Large conversions can take a while and may exceed the MCP client's per-call
  timeout — convert smaller segments or accept the wait.
- The format is chosen by the `output` extension unless you override codecs in `args`.

## See also
[tools.md](../tools.md)
