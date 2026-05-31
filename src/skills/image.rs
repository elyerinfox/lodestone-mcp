//! Image forensics & metadata. Read-only inspection of image files; all
//! paths go through `[filesystem]::resolve` so they're confined to the
//! configured roots. Four tools:
//!
//! * `image_info` — format / dimensions / color / animation flag from the
//!   container's structural headers (JPEG SOFn, PNG IHDR, GIF LSD, WebP VP8,
//!   BMP DIB, TIFF IFD0). Pure binary parsing, no full-image decode.
//! * `image_exif` — full EXIF tag dump including IFD0 / Exif / GPS / Interop.
//!   Decodes GPS lat/lon into signed decimal degrees and renders an
//!   openstreetmap.org link. Flags forensic divergences (DateTimeOriginal
//!   vs DateTime, editor-branded Software tag, etc.).
//! * `image_jpeg_analyze` — every JPEG marker: APP segments (JFIF, EXIF,
//!   XMP, ICC, Photoshop / 8BIM, MPF), comments (COM), quantization tables
//!   (DQT), Huffman tables (DHT), restart intervals, SOFn payload, scan
//!   headers. Useful for camera-vs-editor identification and tamper checks.
//! * `image_png_analyze` — every PNG chunk: IHDR (dims / depth / color
//!   type), tEXt / iTXt / zTXt (textual metadata), eXIf, iCCP (color
//!   profile), tIME (last modification timestamp), pHYs (DPI), gAMA, sRGB.
//!
//! On by default behind `[image].enabled`.

use std::fmt::Write as _;
use std::sync::Arc;

use exif::{In, Reader, Tag, Value};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{fs_read_bytes, schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

pub const TOOL_NAMES: &[&str] = &[
    "image_info",
    "image_exif",
    "image_jpeg_analyze",
    "image_png_analyze",
];

// ---------------------------------------------------------------------------
// image_info — format detection + structural dimensions
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct PathArgs {
    /// Path to the image (confined to `[filesystem].roots`).
    path: String,
}

pub struct ImageInfo;
impl Skill for ImageInfo {
    fn name(&self) -> &'static str {
        "image_info"
    }
    fn description(&self) -> &'static str {
        "Identify an image's format and read its structural headers (no full decode). Returns \
        format, width, height, color components / bit depth where the container reports them, and \
        whether the file is animated. Supports JPEG, PNG, GIF, WebP, BMP, TIFF, HEIF/HEIC \
        (basic), AVIF, JPEG XL. Useful as the first step in any forensic pass."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PathArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PathArgs>()?;
            let (p, bytes) = fs_read_bytes(server, &args.path)?;
            let info = detect_image(&bytes);
            let mut out = format!("{}\n  bytes: {}\n", p.display(), bytes.len());
            match info {
                Some(d) => {
                    let _ = writeln!(out, "  format: {}", d.format);
                    if let Some((w, h)) = d.dims {
                        let _ = writeln!(out, "  dimensions: {w} × {h}");
                    }
                    if let Some(depth) = d.bit_depth {
                        let _ = writeln!(out, "  bit depth: {depth}");
                    }
                    if let Some(comp) = d.components {
                        let _ = writeln!(out, "  components: {comp}");
                    }
                    if let Some(color) = d.color_type {
                        let _ = writeln!(out, "  color type: {color}");
                    }
                    if d.animated {
                        out.push_str("  animated: yes\n");
                    }
                    if !d.notes.is_empty() {
                        let _ = writeln!(out, "  notes: {}", d.notes);
                    }
                }
                None => {
                    out.push_str("  format: unknown (no matching magic bytes)\n");
                }
            }
            Ok(text_result(out))
        })
    }
}

#[derive(Default)]
struct ImageDescr {
    format: String,
    dims: Option<(u32, u32)>,
    bit_depth: Option<u8>,
    components: Option<u8>,
    color_type: Option<String>,
    animated: bool,
    notes: String,
}

fn detect_image(b: &[u8]) -> Option<ImageDescr> {
    if b.len() >= 8 && &b[0..8] == b"\x89PNG\r\n\x1a\n" {
        return Some(parse_png_info(b));
    }
    if b.len() >= 3 && b[0] == 0xff && b[1] == 0xd8 && b[2] == 0xff {
        return Some(parse_jpeg_info(b));
    }
    if b.len() >= 6 && (&b[0..6] == b"GIF87a" || &b[0..6] == b"GIF89a") {
        return Some(parse_gif_info(b));
    }
    if b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Some(parse_webp_info(b));
    }
    if b.len() >= 2 && &b[0..2] == b"BM" {
        return Some(parse_bmp_info(b));
    }
    if b.len() >= 4 && (&b[0..4] == b"II*\0" || &b[0..4] == b"MM\0*") {
        return Some(parse_tiff_info(b));
    }
    if b.len() >= 12 && &b[4..12] == b"ftypheic" || b.len() >= 12 && &b[4..8] == b"ftyp" {
        let brand = String::from_utf8_lossy(&b[8..12]);
        return Some(ImageDescr {
            format: format!("HEIF/ISOBMFF (brand: {brand})"),
            notes: "Full HEIF parsing not implemented; use image_exif for metadata.".into(),
            ..Default::default()
        });
    }
    if b.len() >= 12 && b[0..12] == [0, 0, 0, 0xc, b'J', b'X', b'L', b' ', 0xd, 0xa, 0x87, 0xa] {
        return Some(ImageDescr {
            format: "JPEG XL".into(),
            ..Default::default()
        });
    }
    None
}

fn parse_png_info(b: &[u8]) -> ImageDescr {
    // PNG: 8-byte signature, then IHDR chunk at offset 8.
    // Chunk = 4-byte length, 4-byte type, data, 4-byte CRC.
    if b.len() < 33 || &b[12..16] != b"IHDR" {
        return ImageDescr {
            format: "PNG".into(),
            ..Default::default()
        };
    }
    let w = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
    let h = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
    let depth = b[24];
    let color_type = match b[25] {
        0 => "grayscale",
        2 => "RGB",
        3 => "indexed (palette)",
        4 => "grayscale + alpha",
        6 => "RGBA",
        n => {
            return ImageDescr {
                format: "PNG".into(),
                dims: Some((w, h)),
                bit_depth: Some(depth),
                color_type: Some(format!("unknown ({n})")),
                ..Default::default()
            }
        }
    };
    // APNG detection: look for an acTL chunk anywhere before IDAT.
    let animated = find_chunk_name(b, b"acTL").is_some();
    ImageDescr {
        format: if animated {
            "APNG".into()
        } else {
            "PNG".into()
        },
        dims: Some((w, h)),
        bit_depth: Some(depth),
        color_type: Some(color_type.into()),
        animated,
        ..Default::default()
    }
}

/// Scan PNG chunks for one matching `name`. Returns the offset of the chunk's
/// length field, or `None` if absent.
fn find_chunk_name(b: &[u8], name: &[u8]) -> Option<usize> {
    let mut i = 8;
    while i + 8 <= b.len() {
        let len = u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
        let ctype = &b[i + 4..i + 8];
        if ctype == name {
            return Some(i);
        }
        // Length + type + data + CRC.
        i = i.checked_add(12)?.checked_add(len)?;
    }
    None
}

fn parse_jpeg_info(b: &[u8]) -> ImageDescr {
    // Walk markers until we find SOF0..SOF15 (SOF1=0xc1, etc., excluding
    // 0xc4 = DHT, 0xc8 = JPG-extension, 0xcc = DAC).
    let mut i = 2; // skip SOI (FF D8)
    let mut dims = None;
    let mut bit_depth = None;
    let mut components = None;
    while i + 4 <= b.len() {
        if b[i] != 0xff {
            i += 1;
            continue;
        }
        let marker = b[i + 1];
        i += 2;
        if (0xd0..=0xd9).contains(&marker) {
            // Standalone markers (RSTn, SOI, EOI) — no payload.
            continue;
        }
        if i + 2 > b.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([b[i], b[i + 1]]) as usize;
        if seg_len < 2 || i + seg_len > b.len() {
            break;
        }
        // SOF markers (0xc0..0xcf except 0xc4, 0xc8, 0xcc).
        if (0xc0..=0xcf).contains(&marker)
            && marker != 0xc4
            && marker != 0xc8
            && marker != 0xcc
            && seg_len >= 8
        {
            bit_depth = Some(b[i + 2]);
            let height = u16::from_be_bytes([b[i + 3], b[i + 4]]) as u32;
            let width = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
            dims = Some((width, height));
            components = Some(b[i + 7]);
            break;
        }
        i += seg_len;
    }
    ImageDescr {
        format: "JPEG".into(),
        dims,
        bit_depth,
        components,
        color_type: components.map(|c| match c {
            1 => "grayscale".into(),
            3 => "YCbCr (color)".into(),
            4 => "CMYK / YCCK".into(),
            n => format!("{n} channels"),
        }),
        ..Default::default()
    }
}

fn parse_gif_info(b: &[u8]) -> ImageDescr {
    // After 6-byte header, Logical Screen Descriptor is 7 bytes:
    // width (LE u16), height (LE u16), packed, bg, aspect.
    if b.len() < 13 {
        return ImageDescr {
            format: "GIF".into(),
            ..Default::default()
        };
    }
    let w = u16::from_le_bytes([b[6], b[7]]) as u32;
    let h = u16::from_le_bytes([b[8], b[9]]) as u32;
    // Animated detection: scan for multiple Image Descriptors (0x2c).
    let img_count = b.iter().filter(|&&x| x == 0x2c).count();
    ImageDescr {
        format: "GIF".into(),
        dims: Some((w, h)),
        bit_depth: Some((b[10] & 0x07) + 1),
        animated: img_count > 1,
        notes: if img_count > 1 {
            format!("{img_count} image descriptors")
        } else {
            String::new()
        },
        ..Default::default()
    }
}

fn parse_webp_info(b: &[u8]) -> ImageDescr {
    // RIFF header is 12 bytes. Inside, look for "VP8 ", "VP8L", "VP8X".
    if b.len() < 30 {
        return ImageDescr {
            format: "WebP".into(),
            ..Default::default()
        };
    }
    let chunk_type = &b[12..16];
    let (dims, animated) = match chunk_type {
        b"VP8 " => {
            // Lossy. Width / height at offset 26 (10-bit LE values, lower 14 bits).
            let w = u16::from_le_bytes([b[26], b[27]]) as u32 & 0x3fff;
            let h = u16::from_le_bytes([b[28], b[29]]) as u32 & 0x3fff;
            (Some((w, h)), false)
        }
        b"VP8L" => {
            // Lossless. Width-1 (14 bits) + height-1 (14 bits) at offset 21.
            let s0 = b[21] as u32;
            let s1 = b[22] as u32;
            let s2 = b[23] as u32;
            let s3 = b[24] as u32;
            let w = (s0 | (s1 << 8)) & 0x3fff;
            let h = ((s1 >> 6) | (s2 << 2) | (s3 << 10)) & 0x3fff;
            (Some((w + 1, h + 1)), false)
        }
        b"VP8X" => {
            // Extended. Width-1 (24-bit) + height-1 (24-bit) at offsets 24 and 27.
            if b.len() < 30 {
                (None, false)
            } else {
                let w = (b[24] as u32) | ((b[25] as u32) << 8) | ((b[26] as u32) << 16);
                let h = (b[27] as u32) | ((b[28] as u32) << 8) | ((b[29] as u32) << 16);
                let anim = (b[20] & 0x02) != 0;
                (Some((w + 1, h + 1)), anim)
            }
        }
        _ => (None, false),
    };
    ImageDescr {
        format: "WebP".into(),
        dims,
        animated,
        notes: format!("inner chunk: {}", String::from_utf8_lossy(chunk_type)),
        ..Default::default()
    }
}

fn parse_bmp_info(b: &[u8]) -> ImageDescr {
    // BMP file header 14 bytes, then DIB header. DIB starts at offset 14.
    if b.len() < 30 {
        return ImageDescr {
            format: "BMP".into(),
            ..Default::default()
        };
    }
    let dib_size = u32::from_le_bytes([b[14], b[15], b[16], b[17]]);
    let (w, h, depth) = if dib_size >= 40 {
        let w = i32::from_le_bytes([b[18], b[19], b[20], b[21]]);
        let h = i32::from_le_bytes([b[22], b[23], b[24], b[25]]);
        let depth = u16::from_le_bytes([b[28], b[29]]);
        (w.unsigned_abs(), h.unsigned_abs(), depth as u8)
    } else {
        (0, 0, 0)
    };
    ImageDescr {
        format: "BMP".into(),
        dims: Some((w, h)),
        bit_depth: Some(depth),
        ..Default::default()
    }
}

fn parse_tiff_info(b: &[u8]) -> ImageDescr {
    // TIFF: byte order, magic (42), IFD offset (u32). For brevity, just
    // report the byte order — full TIFF IFD parsing is complex and the
    // kamadak-exif crate handles it for image_exif.
    let bo = if &b[0..4] == b"II*\0" {
        "little-endian (II)"
    } else {
        "big-endian (MM)"
    };
    ImageDescr {
        format: "TIFF".into(),
        notes: format!("byte order: {bo}. Use image_exif for IFD0 / EXIF / GPS tags."),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// image_exif — full EXIF dump including GPS
// ---------------------------------------------------------------------------

pub struct ImageExif;
impl Skill for ImageExif {
    fn name(&self) -> &'static str {
        "image_exif"
    }
    fn description(&self) -> &'static str {
        "Dump every EXIF tag from a JPEG or TIFF image (IFD0, Exif, GPS, Interop). \
        Reports camera make / model, exposure, focal length, ISO, lens, all timestamps \
        (DateTimeOriginal / DateTimeDigitized / DateTime — divergence between these is a \
        forensic signal), software / editor (a non-camera value here flags a tampered or \
        re-saved file), and decoded GPS coordinates as signed decimal degrees with an OSM map \
        link. Bytes returned: a digest, not the raw binary IFDs."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PathArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PathArgs>()?;
            let (p, bytes) = fs_read_bytes(server, &args.path)?;
            let mut cursor = std::io::Cursor::new(&bytes);
            let exif_data = match Reader::new().read_from_container(&mut cursor) {
                Ok(e) => e,
                Err(e) => {
                    return Ok(text_result(format!("{} — no EXIF data ({e})", p.display())));
                }
            };
            let mut out = format!("{} — EXIF\n", p.display());

            // Group by IFD for readability.
            for ifd in [In::PRIMARY, In::THUMBNAIL] {
                let label = if ifd == In::PRIMARY {
                    "primary"
                } else {
                    "thumbnail"
                };
                let mut seen_header = false;
                for field in exif_data.fields().filter(|f| f.ifd_num == ifd) {
                    // Decoded GPS gets a custom rendering below; skip the
                    // raw bytes here so we don't double-print.
                    if matches!(
                        field.tag,
                        Tag::GPSLatitude
                            | Tag::GPSLongitude
                            | Tag::GPSLatitudeRef
                            | Tag::GPSLongitudeRef
                            | Tag::GPSAltitude
                            | Tag::GPSAltitudeRef
                    ) {
                        continue;
                    }
                    if !seen_header {
                        let _ = writeln!(out, "\n[{label}]");
                        seen_header = true;
                    }
                    let display = field.display_value().with_unit(&exif_data).to_string();
                    let _ = writeln!(out, "  {} = {}", field.tag, display);
                }
            }

            // GPS decoded.
            if let Some((lat, lon)) = decode_gps(&exif_data) {
                let _ = writeln!(out, "\n[GPS]");
                let _ = writeln!(out, "  latitude:  {lat:.7}");
                let _ = writeln!(out, "  longitude: {lon:.7}");
                let _ = writeln!(
                    out,
                    "  map: https://www.openstreetmap.org/?mlat={lat:.5}&mlon={lon:.5}#map=15/{lat:.5}/{lon:.5}"
                );
                if let Some(alt) = decode_gps_altitude(&exif_data) {
                    let _ = writeln!(out, "  altitude:  {alt:.2} m");
                }
            }

            // Forensic flags.
            let mut flags: Vec<String> = Vec::new();
            let dto = field_string(&exif_data, In::PRIMARY, Tag::DateTimeOriginal);
            let dtd = field_string(&exif_data, In::PRIMARY, Tag::DateTimeDigitized);
            let dt = field_string(&exif_data, In::PRIMARY, Tag::DateTime);
            if let (Some(a), Some(b)) = (dto.as_deref(), dt.as_deref()) {
                if a != b {
                    flags.push(format!(
                        "DateTimeOriginal ({a}) differs from DateTime ({b}) — \
                         the file has been re-saved or its modification timestamp updated."
                    ));
                }
            }
            if let (Some(a), Some(b)) = (dto.as_deref(), dtd.as_deref()) {
                if a != b {
                    flags.push(format!(
                        "DateTimeOriginal ({a}) differs from DateTimeDigitized ({b}) — \
                         scan / digitization workflow."
                    ));
                }
            }
            if let Some(sw) = field_string(&exif_data, In::PRIMARY, Tag::Software) {
                let sw_lower = sw.to_ascii_lowercase();
                if sw_lower.contains("photoshop")
                    || sw_lower.contains("gimp")
                    || sw_lower.contains("lightroom")
                    || sw_lower.contains("capture one")
                    || sw_lower.contains("affinity")
                    || sw_lower.contains("pixelmator")
                {
                    flags.push(format!(
                        "Software tag is editor-branded ({sw}) — image was processed in a photo editor."
                    ));
                }
            }
            if !flags.is_empty() {
                out.push_str("\n[Forensic flags]\n");
                for f in flags {
                    let _ = writeln!(out, "  ⚠ {f}");
                }
            }
            Ok(text_result(out))
        })
    }
}

fn field_string(e: &exif::Exif, ifd: In, tag: Tag) -> Option<String> {
    e.get_field(tag, ifd).map(|f| {
        f.display_value()
            .with_unit(e)
            .to_string()
            .trim_matches('"')
            .to_string()
    })
}

fn decode_gps(e: &exif::Exif) -> Option<(f64, f64)> {
    let lat = e.get_field(Tag::GPSLatitude, In::PRIMARY)?;
    let lat_ref = e
        .get_field(Tag::GPSLatitudeRef, In::PRIMARY)
        .and_then(|f| match &f.value {
            Value::Ascii(v) if !v.is_empty() => Some(v[0].first().copied().unwrap_or(b'N')),
            _ => None,
        })
        .unwrap_or(b'N');
    let lon = e.get_field(Tag::GPSLongitude, In::PRIMARY)?;
    let lon_ref = e
        .get_field(Tag::GPSLongitudeRef, In::PRIMARY)
        .and_then(|f| match &f.value {
            Value::Ascii(v) if !v.is_empty() => Some(v[0].first().copied().unwrap_or(b'E')),
            _ => None,
        })
        .unwrap_or(b'E');
    let dms_to_dd = |v: &exif::Field| -> Option<f64> {
        if let Value::Rational(rs) = &v.value {
            if rs.len() == 3 {
                let d = rs[0].to_f64();
                let m = rs[1].to_f64();
                let s = rs[2].to_f64();
                return Some(d + m / 60.0 + s / 3600.0);
            }
        }
        None
    };
    let lat_dd = dms_to_dd(lat)?;
    let lon_dd = dms_to_dd(lon)?;
    let lat_signed = if matches!(lat_ref, b'S' | b's') {
        -lat_dd
    } else {
        lat_dd
    };
    let lon_signed = if matches!(lon_ref, b'W' | b'w') {
        -lon_dd
    } else {
        lon_dd
    };
    Some((lat_signed, lon_signed))
}

fn decode_gps_altitude(e: &exif::Exif) -> Option<f64> {
    let alt = e.get_field(Tag::GPSAltitude, In::PRIMARY)?;
    let alt_ref = e
        .get_field(Tag::GPSAltitudeRef, In::PRIMARY)
        .and_then(|f| match &f.value {
            Value::Byte(v) if !v.is_empty() => Some(v[0]),
            _ => None,
        })
        .unwrap_or(0);
    let v = match &alt.value {
        Value::Rational(rs) if !rs.is_empty() => rs[0].to_f64(),
        _ => return None,
    };
    Some(if alt_ref == 1 { -v } else { v })
}

// ---------------------------------------------------------------------------
// image_jpeg_analyze — every marker, with focus on forensically-useful ones
// ---------------------------------------------------------------------------

pub struct ImageJpegAnalyze;
impl Skill for ImageJpegAnalyze {
    fn name(&self) -> &'static str {
        "image_jpeg_analyze"
    }
    fn description(&self) -> &'static str {
        "Walk every JPEG marker in the file and report it. Identifies APP0 (JFIF), APP1 (EXIF \
        and / or XMP), APP2 (ICC color profile, MPF multi-picture), APP13 (Photoshop / 8BIM), \
        APP14 (Adobe), COM (comments), DQT (quantization tables — fingerprint of the encoder, \
        camera-vs-editor identification), DHT (Huffman tables), DRI (restart interval), SOFn \
        (frame parameters), SOS (scan). Useful for tamper detection and source-camera \
        attribution."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PathArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PathArgs>()?;
            let (p, bytes) = fs_read_bytes(server, &args.path)?;
            if bytes.len() < 2 || bytes[0] != 0xff || bytes[1] != 0xd8 {
                return Err(invalid(format!(
                    "{} — not a JPEG (no SOI marker)",
                    p.display()
                )));
            }
            let mut out = format!("{} — JPEG markers\n", p.display());
            let mut i = 2;
            let mut dqt_count = 0;
            let mut dht_count = 0;
            while i + 4 <= bytes.len() {
                if bytes[i] != 0xff {
                    i += 1;
                    continue;
                }
                let marker = bytes[i + 1];
                if marker == 0x00 || marker == 0xff {
                    i += 2;
                    continue;
                }
                i += 2;
                if marker == 0xd9 {
                    out.push_str("  FFD9  EOI (end of image)\n");
                    break;
                }
                if (0xd0..=0xd9).contains(&marker) {
                    let _ = writeln!(out, "  FF{:02X}  RST{}", marker, marker & 0x0f);
                    continue;
                }
                if i + 2 > bytes.len() {
                    break;
                }
                let seg_len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
                if seg_len < 2 || i + seg_len > bytes.len() {
                    let _ = writeln!(
                        out,
                        "  FF{:02X}  (truncated, seg_len={seg_len}, remaining={})",
                        marker,
                        bytes.len() - i
                    );
                    break;
                }
                let payload = &bytes[i + 2..i + seg_len];
                let (name, detail) =
                    describe_jpeg_marker(marker, payload, &mut dqt_count, &mut dht_count);
                let _ = writeln!(
                    out,
                    "  FF{:02X}  {:<8} len={:>5}  {detail}",
                    marker, name, seg_len
                );
                i += seg_len;
                if marker == 0xda {
                    // SOS — entropy-coded data follows; skip to the next FFxx
                    // marker that isn't FF00 / FFFF.
                    while i + 1 < bytes.len() {
                        if bytes[i] == 0xff && bytes[i + 1] != 0x00 && bytes[i + 1] != 0xff {
                            break;
                        }
                        i += 1;
                    }
                }
            }
            Ok(text_result(out))
        })
    }
}

fn describe_jpeg_marker(
    marker: u8,
    payload: &[u8],
    dqt_count: &mut u32,
    dht_count: &mut u32,
) -> (&'static str, String) {
    match marker {
        0xc0 => ("SOF0", sof_detail(payload)),
        0xc1 => ("SOF1", sof_detail(payload)),
        0xc2 => ("SOF2", sof_detail(payload)),
        0xc3 => ("SOF3", sof_detail(payload)),
        0xc4 => {
            *dht_count += 1;
            (
                "DHT",
                format!(
                    "Huffman table #{} (class+id={:#04x})",
                    dht_count,
                    payload.first().copied().unwrap_or(0)
                ),
            )
        }
        0xc5..=0xcf if marker != 0xc8 && marker != 0xcc => ("SOFn", sof_detail(payload)),
        0xd8 => ("SOI", String::new()),
        0xd9 => ("EOI", String::new()),
        0xda => (
            "SOS",
            format!(
                "scan, {} component(s)",
                payload.first().copied().unwrap_or(0)
            ),
        ),
        0xdb => {
            *dqt_count += 1;
            let pq = payload.first().copied().unwrap_or(0);
            (
                "DQT",
                format!(
                    "quantization table #{} (Pq={:#x}, Tq={})",
                    dqt_count,
                    pq >> 4,
                    pq & 0x0f
                ),
            )
        }
        0xdd => (
            "DRI",
            format!(
                "restart interval = {}",
                u16::from_be_bytes([
                    payload.first().copied().unwrap_or(0),
                    payload.get(1).copied().unwrap_or(0)
                ])
            ),
        ),
        0xe0 => app_detail("APP0", payload),
        0xe1 => app_detail("APP1", payload),
        0xe2 => app_detail("APP2", payload),
        0xed => app_detail("APP13", payload),
        0xee => app_detail("APP14", payload),
        0xe3..=0xef => app_detail("APPn", payload),
        0xfe => (
            "COM",
            format!(
                "comment: {:?}",
                String::from_utf8_lossy(&payload[..payload.len().min(120)])
            ),
        ),
        _ => ("?", String::new()),
    }
}

fn sof_detail(payload: &[u8]) -> String {
    if payload.len() < 6 {
        return "(short)".into();
    }
    let depth = payload[0];
    let height = u16::from_be_bytes([payload[1], payload[2]]);
    let width = u16::from_be_bytes([payload[3], payload[4]]);
    let components = payload[5];
    format!(
        "{}×{}, depth={}, components={}",
        width, height, depth, components
    )
}

fn app_detail(name: &'static str, payload: &[u8]) -> (&'static str, String) {
    // APP segments use a NUL-terminated identifier string as their header.
    let id_end = payload
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(payload.len());
    let id = String::from_utf8_lossy(&payload[..id_end.min(16)]).to_string();
    let detail = match id.as_str() {
        "JFIF" | "JFXX" => format!("identifier: {id} (JFIF / JFIF extension)"),
        "Exif" => "identifier: Exif (TIFF-encoded metadata)".into(),
        "http://ns.adobe.com/xap/1.0/" => "identifier: XMP".into(),
        "ICC_PROFILE" => "identifier: ICC_PROFILE (color profile)".into(),
        "MPF" => "identifier: MPF (multi-picture format — embedded preview)".into(),
        "Photoshop 3.0" => "identifier: Photoshop 3.0 (8BIM resource blocks)".into(),
        "Adobe" => "identifier: Adobe (color transform marker)".into(),
        s if s.starts_with("http://ns.adobe.com/xmp/extension/") => {
            "identifier: XMP-Extension".into()
        }
        _ => format!("identifier: {id:?}"),
    };
    (name, detail)
}

// ---------------------------------------------------------------------------
// image_png_analyze — every chunk
// ---------------------------------------------------------------------------

pub struct ImagePngAnalyze;
impl Skill for ImagePngAnalyze {
    fn name(&self) -> &'static str {
        "image_png_analyze"
    }
    fn description(&self) -> &'static str {
        "Walk every PNG chunk in the file. Reports IHDR (width / height / depth / color type / \
        filter / interlace), tEXt / iTXt / zTXt (textual metadata — software, comments, \
        copyright), eXIf (PNG-embedded EXIF), iCCP (color profile), tIME (last-modified \
        timestamp), pHYs (physical pixel dimensions / DPI), gAMA, sRGB, acTL (animation), and \
        flags any unknown private chunks."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<PathArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<PathArgs>()?;
            let (p, bytes) = fs_read_bytes(server, &args.path)?;
            if bytes.len() < 8 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
                return Err(invalid(format!(
                    "{} — not a PNG (no signature)",
                    p.display()
                )));
            }
            let mut out = format!("{} — PNG chunks\n", p.display());
            let mut i = 8;
            while i + 12 <= bytes.len() {
                let len = u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]])
                    as usize;
                let ctype = &bytes[i + 4..i + 8];
                let ctype_str = std::str::from_utf8(ctype).unwrap_or("????");
                let data_end = i + 8 + len;
                if data_end + 4 > bytes.len() {
                    let _ = writeln!(out, "  {ctype_str}  (truncated)");
                    break;
                }
                let data = &bytes[i + 8..data_end];
                let detail = describe_png_chunk(ctype, data);
                let _ = writeln!(out, "  {ctype_str}  len={:>6}  {detail}", len);
                i = data_end + 4; // skip CRC
                if ctype == b"IEND" {
                    break;
                }
            }
            Ok(text_result(out))
        })
    }
}

fn describe_png_chunk(ctype: &[u8], data: &[u8]) -> String {
    match ctype {
        b"IHDR" if data.len() >= 13 => {
            let w = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            let h = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
            let color = match data[9] {
                0 => "gray",
                2 => "RGB",
                3 => "palette",
                4 => "gray+A",
                6 => "RGBA",
                _ => "?",
            };
            let interlace = if data[12] == 0 { "none" } else { "Adam7" };
            format!(
                "{w}×{h} depth={} color={color} compression={} filter={} interlace={interlace}",
                data[8], data[10], data[11]
            )
        }
        b"tEXt" => {
            // keyword\0text
            let nul = data.iter().position(|&b| b == 0).unwrap_or(data.len());
            let k = String::from_utf8_lossy(&data[..nul]);
            let v = if nul + 1 < data.len() {
                String::from_utf8_lossy(&data[nul + 1..]).to_string()
            } else {
                String::new()
            };
            let v_trim: String = v.chars().take(120).collect();
            format!("{k} = {v_trim:?}")
        }
        b"iTXt" => {
            // keyword\0compression_flag\0compression_method\0lang\0translated_keyword\0text
            let mut parts = data.split(|&b| b == 0);
            let keyword = parts
                .next()
                .map(|p| String::from_utf8_lossy(p).to_string())
                .unwrap_or_default();
            format!("(international) {keyword}")
        }
        b"zTXt" => "(zlib-compressed text)".into(),
        b"tIME" if data.len() >= 7 => {
            let y = u16::from_be_bytes([data[0], data[1]]);
            format!(
                "last-modified {y}-{:02}-{:02} {:02}:{:02}:{:02}",
                data[2], data[3], data[4], data[5], data[6]
            )
        }
        b"pHYs" if data.len() >= 9 => {
            let ppu_x = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            let ppu_y = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
            let unit = if data[8] == 1 { "meter" } else { "unknown" };
            // 1 inch = 0.0254 m
            let dpi_x = ppu_x as f64 * 0.0254;
            let dpi_y = ppu_y as f64 * 0.0254;
            if data[8] == 1 {
                format!("{ppu_x}×{ppu_y} pixels/{unit} (~{dpi_x:.0}×{dpi_y:.0} DPI)")
            } else {
                format!("{ppu_x}×{ppu_y} pixels/{unit}")
            }
        }
        b"gAMA" if data.len() >= 4 => {
            let g = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            format!("gamma = {:.4}", g as f64 / 100_000.0)
        }
        b"sRGB" if !data.is_empty() => format!("rendering intent {}", data[0]),
        b"iCCP" => "ICC color profile (zlib-compressed)".into(),
        b"eXIf" => format!(
            "embedded EXIF, {} bytes (use image_exif for decoded tags)",
            data.len()
        ),
        b"acTL" if data.len() >= 8 => {
            let frames = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
            let plays = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
            let plays_disp = if plays == 0 {
                "loop forever".into()
            } else {
                format!("{plays} play(s)")
            };
            format!("APNG: {frames} frames, {plays_disp}")
        }
        b"IDAT" => "(image data)".into(),
        b"IEND" => "(end)".into(),
        _ => {
            let private = (ctype[0] & 0x20) != 0;
            if private {
                format!("(private chunk, {} bytes)", data.len())
            } else {
                format!("({} bytes)", data.len())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(ImageInfo),
        Box::new(ImageExif),
        Box::new(ImageJpegAnalyze),
        Box::new(ImagePngAnalyze),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal 1×1 PNG: signature + IHDR + IDAT + IEND. Used to test the
    /// `image_info` path without a real file.
    fn minimal_png() -> Vec<u8> {
        // Hand-crafted with valid CRCs (pre-computed).
        // 89 50 4E 47 0D 0A 1A 0A  signature
        // 00 00 00 0D IHDR length
        // 49 48 44 52  IHDR
        // 00 00 00 01 00 00 00 01 08 06 00 00 00  width=1, height=1, depth=8, color=6, ...
        // 1F 15 C4 89  IHDR CRC
        // 00 00 00 0A IDAT length
        // 49 44 41 54  IDAT
        // 78 9C 63 00 01 00 00 05 00 01  zlib stream of 1 transparent pixel
        // 0D 0A 2D B4  IDAT CRC
        // 00 00 00 00 IEND length
        // 49 45 4E 44  IEND
        // AE 42 60 82  IEND CRC
        vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ]
    }

    #[test]
    fn detect_png_basics() {
        let b = minimal_png();
        let d = detect_image(&b).unwrap();
        assert_eq!(d.format, "PNG");
        assert_eq!(d.dims, Some((1, 1)));
        assert_eq!(d.bit_depth, Some(8));
        assert_eq!(d.color_type.as_deref(), Some("RGBA"));
        assert!(!d.animated);
    }

    #[test]
    fn detect_jpeg_soi() {
        let b = vec![0xff, 0xd8, 0xff, 0xe0];
        let d = detect_image(&b).unwrap();
        assert_eq!(d.format, "JPEG");
        // SOF wasn't reached → no dims; that's expected and not a failure.
    }

    #[test]
    fn detect_gif_signature() {
        let mut b = b"GIF89a".to_vec();
        b.extend_from_slice(&[
            0x40, 0x00, // width=64
            0x30, 0x00, // height=48
            0xf7, 0x00, 0x00,
        ]);
        let d = detect_image(&b).unwrap();
        assert_eq!(d.format, "GIF");
        assert_eq!(d.dims, Some((64, 48)));
    }

    #[test]
    fn detect_bmp_signature() {
        let mut b = b"BM".to_vec();
        b.resize(30, 0);
        // dib_size = 40 at offset 14
        b[14..18].copy_from_slice(&40u32.to_le_bytes());
        b[18..22].copy_from_slice(&320i32.to_le_bytes());
        b[22..26].copy_from_slice(&240i32.to_le_bytes());
        b[28..30].copy_from_slice(&24u16.to_le_bytes());
        let d = detect_image(&b).unwrap();
        assert_eq!(d.format, "BMP");
        assert_eq!(d.dims, Some((320, 240)));
        assert_eq!(d.bit_depth, Some(24));
    }

    #[test]
    fn unknown_magic_returns_none() {
        assert!(detect_image(b"not an image at all").is_none());
    }
}
