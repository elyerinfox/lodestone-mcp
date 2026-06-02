//! Geospatial format converters — NMEA-0183 sentence decode, Cursor-on-Target
//! XML encode, GeoJSON↔WKT.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::json;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct NmeaArgs {
    /// Raw NMEA-0183 sentence (e.g. `$GPGGA,...*hh`).
    sentence: String,
}

pub struct ConvertNmeaDecode;
impl Skill for ConvertNmeaDecode {
    fn name(&self) -> &'static str {
        "convert_nmea_decode"
    }
    fn description(&self) -> &'static str {
        "Parse a NMEA-0183 sentence (GPGGA, GPRMC, GPGSA, GPGSV, GPVTG, …) \
        into a structured JSON object with the named fields. Verifies the \
        XOR checksum at the tail."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NmeaArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<NmeaArgs>()?;
            let s = a.sentence.trim();
            if !s.starts_with('$') {
                return Err(invalid("NMEA sentence must start with '$'"));
            }
            // Verify checksum.
            let star = s.rfind('*').ok_or_else(|| invalid("missing checksum delimiter '*'"))?;
            let body = &s[1..star];
            let expected = &s[star + 1..];
            let mut chk: u8 = 0;
            for b in body.as_bytes() {
                chk ^= b;
            }
            let actual = format!("{chk:02X}");
            if !actual.eq_ignore_ascii_case(expected.split_whitespace().next().unwrap_or("")) {
                return Err(invalid(format!(
                    "checksum mismatch: computed {actual}, sentence claims {expected}"
                )));
            }
            let parts: Vec<&str> = body.split(',').collect();
            let sentence_type = parts[0];
            let fields = match &sentence_type[2..] {
                "GGA" => parse_gga(&parts),
                "RMC" => parse_rmc(&parts),
                "GSA" => parse_gsa(&parts),
                "GSV" => parse_gsv(&parts),
                "VTG" => parse_vtg(&parts),
                _ => json!({ "raw": parts }),
            };
            Ok(text_result(
                json!({ "sentence_type": sentence_type, "fields": fields }).to_string(),
            ))
        })
    }
}

fn nmea_coord(s: &str, hemi: &str) -> Option<f64> {
    if s.is_empty() {
        return None;
    }
    let dot = s.find('.')?;
    let deg_end = dot.saturating_sub(2);
    let deg: f64 = s[..deg_end].parse().ok()?;
    let min: f64 = s[deg_end..].parse().ok()?;
    let sign = if hemi == "S" || hemi == "W" { -1.0 } else { 1.0 };
    Some(sign * (deg + min / 60.0))
}

fn parse_gga(p: &[&str]) -> serde_json::Value {
    json!({
        "utc_time": p.get(1).unwrap_or(&""),
        "lat": nmea_coord(p.get(2).unwrap_or(&""), p.get(3).unwrap_or(&"")),
        "lon": nmea_coord(p.get(4).unwrap_or(&""), p.get(5).unwrap_or(&"")),
        "fix_quality": p.get(6).and_then(|v| v.parse::<i64>().ok()),
        "satellites_used": p.get(7).and_then(|v| v.parse::<i64>().ok()),
        "hdop": p.get(8).and_then(|v| v.parse::<f64>().ok()),
        "altitude_m": p.get(9).and_then(|v| v.parse::<f64>().ok()),
        "geoid_sep_m": p.get(11).and_then(|v| v.parse::<f64>().ok()),
    })
}

fn parse_rmc(p: &[&str]) -> serde_json::Value {
    json!({
        "utc_time": p.get(1).unwrap_or(&""),
        "status": p.get(2).unwrap_or(&""),
        "lat": nmea_coord(p.get(3).unwrap_or(&""), p.get(4).unwrap_or(&"")),
        "lon": nmea_coord(p.get(5).unwrap_or(&""), p.get(6).unwrap_or(&"")),
        "speed_knots": p.get(7).and_then(|v| v.parse::<f64>().ok()),
        "course_deg": p.get(8).and_then(|v| v.parse::<f64>().ok()),
        "date": p.get(9).unwrap_or(&""),
    })
}

fn parse_gsa(p: &[&str]) -> serde_json::Value {
    json!({
        "mode_1": p.get(1).unwrap_or(&""),
        "mode_2": p.get(2).unwrap_or(&""),
        "satellites": p.iter().skip(3).take(12).copied().collect::<Vec<_>>(),
        "pdop": p.get(15).and_then(|v| v.parse::<f64>().ok()),
        "hdop": p.get(16).and_then(|v| v.parse::<f64>().ok()),
        "vdop": p.get(17).and_then(|v| v.parse::<f64>().ok()),
    })
}

fn parse_gsv(p: &[&str]) -> serde_json::Value {
    json!({
        "total_messages": p.get(1).and_then(|v| v.parse::<i64>().ok()),
        "message_number": p.get(2).and_then(|v| v.parse::<i64>().ok()),
        "satellites_in_view": p.get(3).and_then(|v| v.parse::<i64>().ok()),
        "raw_sat_records": p.iter().skip(4).copied().collect::<Vec<_>>(),
    })
}

fn parse_vtg(p: &[&str]) -> serde_json::Value {
    json!({
        "true_course_deg": p.get(1).and_then(|v| v.parse::<f64>().ok()),
        "magnetic_course_deg": p.get(3).and_then(|v| v.parse::<f64>().ok()),
        "speed_knots": p.get(5).and_then(|v| v.parse::<f64>().ok()),
        "speed_kmh": p.get(7).and_then(|v| v.parse::<f64>().ok()),
    })
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CotArgs {
    /// Event UID (unique per emitter).
    uid: String,
    /// CoT type (e.g. `a-f-G-U-C` = friendly ground unit combat).
    cot_type: String,
    lat: f64,
    lon: f64,
    /// Height above ellipsoid (m).
    #[serde(default)]
    hae_m: Option<f64>,
    /// Stale time in seconds after start.
    #[serde(default)]
    stale_seconds: Option<u64>,
    /// Optional callsign for the contact.
    #[serde(default)]
    callsign: Option<String>,
}

pub struct ConvertCotEncode;
impl Skill for ConvertCotEncode {
    fn name(&self) -> &'static str {
        "convert_cot_encode"
    }
    fn description(&self) -> &'static str {
        "Encode a Cursor-on-Target (CoT) event as XML. Default stale window \
        is 60 s. The result is the canonical TAK-ingestible string with a \
        `point`, `detail/contact/callsign` (when provided), and standard \
        time stamps."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CotArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<CotArgs>()?;
            let now = chrono::Utc::now();
            let stale = now + chrono::Duration::seconds(a.stale_seconds.unwrap_or(60) as i64);
            let cs = a
                .callsign
                .as_deref()
                .map(|c| format!("<detail><contact callsign=\"{c}\"/></detail>"))
                .unwrap_or_else(|| "<detail/>".into());
            let xml = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
                <event version=\"2.0\" uid=\"{uid}\" type=\"{typ}\" \
                time=\"{t}\" start=\"{t}\" stale=\"{st}\" how=\"m-g\">\
                <point lat=\"{lat}\" lon=\"{lon}\" hae=\"{hae}\" ce=\"9999999.0\" le=\"9999999.0\"/>\
                {cs}\
                </event>",
                uid = a.uid,
                typ = a.cot_type,
                t = now.to_rfc3339(),
                st = stale.to_rfc3339(),
                lat = a.lat,
                lon = a.lon,
                hae = a.hae_m.unwrap_or(0.0),
                cs = cs,
            );
            Ok(text_result(json!({ "xml": xml }).to_string()))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WktFromGeoJsonArgs {
    /// GeoJSON geometry object (or a GeoJSON Feature — we'll extract its geometry).
    geojson: serde_json::Value,
}

pub struct ConvertGeoJsonToWkt;
impl Skill for ConvertGeoJsonToWkt {
    fn name(&self) -> &'static str {
        "convert_geojson_to_wkt"
    }
    fn description(&self) -> &'static str {
        "Convert a GeoJSON Geometry (Point / LineString / Polygon / Multi*) \
        to Well-Known Text. If a Feature is provided, the geometry is \
        extracted from `feature.geometry`."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<WktFromGeoJsonArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_s, a) = ctx.parse::<WktFromGeoJsonArgs>()?;
            let geom = if a.geojson.get("type").and_then(|t| t.as_str()) == Some("Feature") {
                a.geojson.get("geometry").cloned().unwrap_or(serde_json::Value::Null)
            } else {
                a.geojson.clone()
            };
            let wkt = geojson_to_wkt(&geom)?;
            Ok(text_result(json!({ "wkt": wkt }).to_string()))
        })
    }
}

fn coord_to_wkt(c: &serde_json::Value) -> std::result::Result<String, McpError> {
    let arr = c.as_array().ok_or_else(|| invalid("expected array coord"))?;
    let nums: Vec<String> = arr
        .iter()
        .map(|v| v.as_f64().map(|x| format!("{x}")).unwrap_or_default())
        .collect();
    Ok(nums.join(" "))
}

fn ring_to_wkt(ring: &serde_json::Value) -> std::result::Result<String, McpError> {
    let pts = ring.as_array().ok_or_else(|| invalid("ring expected array"))?;
    let s: std::result::Result<Vec<String>, McpError> =
        pts.iter().map(coord_to_wkt).collect();
    Ok(format!("({})", s?.join(", ")))
}

fn geojson_to_wkt(g: &serde_json::Value) -> std::result::Result<String, McpError> {
    let kind = g
        .get("type")
        .and_then(|t| t.as_str())
        .ok_or_else(|| invalid("missing geometry type"))?;
    let coords = g
        .get("coordinates")
        .ok_or_else(|| invalid("missing coordinates"))?;
    match kind {
        "Point" => Ok(format!("POINT ({})", coord_to_wkt(coords)?)),
        "LineString" => {
            let pts = coords.as_array().ok_or_else(|| invalid("ls expects array"))?;
            let strs: std::result::Result<Vec<String>, McpError> =
                pts.iter().map(coord_to_wkt).collect();
            Ok(format!("LINESTRING ({})", strs?.join(", ")))
        }
        "Polygon" => {
            let rings = coords.as_array().ok_or_else(|| invalid("poly expects array"))?;
            let strs: std::result::Result<Vec<String>, McpError> =
                rings.iter().map(ring_to_wkt).collect();
            Ok(format!("POLYGON ({})", strs?.join(", ")))
        }
        "MultiPoint" => {
            let pts = coords.as_array().ok_or_else(|| invalid("mp expects array"))?;
            let strs: std::result::Result<Vec<String>, McpError> =
                pts.iter().map(|p| coord_to_wkt(p).map(|s| format!("({s})"))).collect();
            Ok(format!("MULTIPOINT ({})", strs?.join(", ")))
        }
        "MultiLineString" => {
            let lines = coords.as_array().ok_or_else(|| invalid("mls expects array"))?;
            let strs: std::result::Result<Vec<String>, McpError> = lines
                .iter()
                .map(|l| {
                    let pts = l.as_array().ok_or_else(|| invalid("mls inner"))?;
                    let s: std::result::Result<Vec<String>, McpError> =
                        pts.iter().map(coord_to_wkt).collect();
                    Ok(format!("({})", s?.join(", ")))
                })
                .collect();
            Ok(format!("MULTILINESTRING ({})", strs?.join(", ")))
        }
        "MultiPolygon" => {
            let polys = coords.as_array().ok_or_else(|| invalid("mpoly expects array"))?;
            let strs: std::result::Result<Vec<String>, McpError> = polys
                .iter()
                .map(|p| {
                    let rings = p.as_array().ok_or_else(|| invalid("mpoly inner"))?;
                    let s: std::result::Result<Vec<String>, McpError> =
                        rings.iter().map(ring_to_wkt).collect();
                    Ok(format!("({})", s?.join(", ")))
                })
                .collect();
            Ok(format!("MULTIPOLYGON ({})", strs?.join(", ")))
        }
        other => Err(invalid(format!("unsupported geometry type '{other}'"))),
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(ConvertNmeaDecode),
        Box::new(ConvertCotEncode),
        Box::new(ConvertGeoJsonToWkt),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nmea_coord_parse() {
        // 47°33.2' N → 47.5533333
        assert!((nmea_coord("4733.2", "N").unwrap() - 47.553_333_333).abs() < 1e-6);
    }
}
