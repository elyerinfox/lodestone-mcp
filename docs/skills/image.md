# Image forensics + EXIF — `image_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/image.rs`](../../src/skills/image.rs) |
| **Tools** | `image_info`, `image_exif`, `image_jpeg_analyze`, `image_png_analyze` |
| **Network** | none — pure-Rust binary parsing |
| **Default** | **on** — gated by `[image]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[image].enabled` via `LODESTONE_IMAGE_ENABLED`. Paths confined to `[filesystem].roots`. Defaults in [`src/config.rs`](../../src/config.rs). |
| **Dep** | `kamadak-exif = "0.6"` (pure Rust, no native deps) |

## What it does

Read-only forensic / metadata inspection of image files. **No full-image
decode** — every tool parses the container's structural headers only, so it's
fast and won't accidentally re-encode anything.

- **`image_info`** identifies the format and pulls dimensions / color /
  animation flags from container headers: JPEG SOFn (`SOF0`/`SOF2`/`SOF3` —
  reads sample-precision, height, width, components), PNG IHDR, GIF LSD,
  WebP VP8 / VP8L / VP8X, BMP DIB header, TIFF magic, HEIF brand boxes,
  JPEG-XL signature.
- **`image_exif`** dumps every EXIF tag from IFD0 / Exif / GPS / Interop via
  `kamadak-exif`. GPS coordinates are decoded to signed decimal degrees with
  an OSM map link. **Forensic divergence flags** fire when
  `DateTimeOriginal` / `DateTime` / `DateTimeDigitized` disagree (re-save /
  scan workflow indicator) or when the `Software` tag is editor-branded
  (Photoshop, GIMP, Lightroom, Capture One, Affinity, Pixelmator). Camera
  files have all three timestamps matching and an empty `Software`; an
  edited file usually doesn't.
- **`image_jpeg_analyze`** walks every JPEG marker in the file:
  - **APP segments by identifier** — JFIF, Exif, XMP, ICC_PROFILE, MPF,
    Photoshop (`8BIM`), Adobe.
  - **DQT** (quantization tables — an encoder fingerprint, varies between
    Photoshop / GIMP / camera firmware).
  - **DHT** counts, **DRI**, **SOFn** payload (dims / depth / components),
    **SOS** scan header.
- **`image_png_analyze`** walks every PNG chunk with decoded payloads: IHDR,
  tEXt / iTXt / zTXt (textual metadata — software, comments), eXIf, iCCP,
  tIME, pHYs (with DPI conversion), gAMA, sRGB, acTL (APNG animation
  control). Unknown private chunks are flagged.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `image_info` | `path` | Format / dimensions / color / animation from structural headers. |
| `image_exif` | `path` | Full EXIF dump with GPS → decimal degrees + OSM link; divergence flags. |
| `image_jpeg_analyze` | `path` | Walk every JPEG marker. |
| `image_png_analyze` | `path` | Walk every PNG chunk with decoded payloads. |

All four take a single `path` argument, confined to `[filesystem].roots`. No
options — the output is the full structural walk.

## Example uses

- **Source identification** — `image_jpeg_analyze` on a suspect photo: the
  DQT pattern + APP segment order plus `image_exif`'s `Software` tag usually
  pin down "camera vs. Photoshop vs. WhatsApp export".
- **Tamper check** — `image_exif` on a "this is the original camera file": if
  `DateTimeOriginal` and `DateTime` disagree, or `Software` is editor-branded,
  the file has been through a save pass.
- **APNG detection** — `image_png_analyze` flags the `acTL` chunk.
- **Color-managed workflow** — `image_png_analyze` decodes the `iCCP` profile
  name and `gAMA` / `sRGB` chunks; `image_jpeg_analyze` reports the
  `ICC_PROFILE` APP segment.

## Notes

- **Read-only.** None of these modify a file.
- **No full decode**, so a corrupted or huge image is parsed in microseconds.
- **Embedded thumbnail extraction** isn't currently exposed —
  `kamadak-exif::Exif` doesn't surface thumbnail bytes; lifting them would
  require manual EXIF byte slicing.

## See also

- [tools.md](../tools.md)
- [skills/filesystem.md](filesystem.md) — `[filesystem].roots` confinement.
- [skills/ffmpeg.md](ffmpeg.md) — for `.mov` / `.mp4` containers, `ffmpeg_probe`
  is the equivalent.
