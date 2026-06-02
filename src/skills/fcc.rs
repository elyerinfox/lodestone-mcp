//! FCC / radio reference skills. Three tools:
//!
//! * `fcc_callsign` — US amateur callsign lookup via the keyless callook.info
//!   JSON API. data.fcc.gov ULS is the official source but is HTTP/2-flaky
//!   from many networks; callook.info wraps the same FCC dataset for amateur
//!   and stays reliably reachable. Non-amateur (GMRS, commercial, broadcast)
//!   gets a friendly note pointing at the FCC ULS web search.
//! * `fcc_amateur_bands` — US amateur band plan with per-license-class
//!   privileges. Baked-in regulatory reference; no network call.
//! * `fcc_radio_service` — non-amateur personal radio services (FRS, GMRS,
//!   MURS, CB), plus a brief unified compare. Baked-in.
//!
//! All three are read-only. On by default behind `[fcc].enabled`.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

// ---------------------------------------------------------------------------
// fcc_callsign — live ULS lookup
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct CallsignArgs {
    /// FCC callsign to look up (e.g. "W1AW", "KE8XYZ", "WPGS123"). Case
    /// insensitive; FCC normalizes to uppercase.
    callsign: String,
}

pub struct FccCallsign;
impl Skill for FccCallsign {
    fn name(&self) -> &'static str {
        "fcc_callsign"
    }
    fn description(&self) -> &'static str {
        "Look up a US amateur radio callsign via the public, keyless callook.info JSON API. \
        Returns licensee name, license class (Technician / General / Amateur Extra), trustee (for \
        club calls), status (VALID / EXPIRED / UPDATING), grant and expiry dates, FRN, address, \
        and the grid square. For non-amateur services (GMRS WQ*/WR*, commercial, broadcast) the \
        tool returns a friendly note pointing at the FCC ULS web search since callook.info covers \
        only the amateur radio service."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<CallsignArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<CallsignArgs>()?;
            let call = args.callsign.trim().to_ascii_uppercase();
            if call.is_empty() {
                return Err(invalid("callsign must not be empty".to_string()));
            }
            // callook.info covers US amateur radio only. It's small, fast,
            // and reliably keyless. data.fcc.gov ULS is flaky / HTTP/2 reset
            // -prone, so we keep that as a manual web-search fallback.
            let url = format!("https://callook.info/{}/json", urlencoded(&call));
            let resp = server
                .http
                .get(&url)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| internal(e.into()))?;
            if resp.status() == reqwest::StatusCode::NOT_FOUND {
                return Ok(text_result(format!(
                    "callook.info has no record for {call}. \
                     If this is a GMRS (WQ*/WR*) or commercial callsign, look it up in the FCC ULS web \
                     search: https://wireless2.fcc.gov/UlsApp/UlsSearch/searchLicense.jsp?searchType=Call+Sign&searchValue={call}\n\n\
                     If it's an amateur callsign you expected to find, double-check spelling — \
                     callook.info covers the entire US amateur radio service."
                )));
            }
            let v: serde_json::Value = resp
                .error_for_status()
                .map_err(|e| internal(e.into()))?
                .json()
                .await
                .map_err(|e| internal(e.into()))?;

            let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("");
            if status.eq_ignore_ascii_case("INVALID") {
                return Ok(text_result(format!(
                    "callook.info says {call} is INVALID (no current US amateur license). \
                     If you expected a GMRS or commercial result, use the FCC ULS web search: \
                     https://wireless2.fcc.gov/UlsApp/UlsSearch/searchLicense.jsp?searchType=Call+Sign&searchValue={call}"
                )));
            }

            let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let r_call = v
                .get("current")
                .and_then(|c| c.get("callsign"))
                .and_then(|x| x.as_str())
                .unwrap_or(&call);
            let op_class = v
                .get("current")
                .and_then(|c| c.get("operClass"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let trustee_call = v
                .get("trustee")
                .and_then(|t| t.get("callsign"))
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty());
            let trustee_name = v
                .get("trustee")
                .and_then(|t| t.get("name"))
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty());
            let prev_class = v
                .get("previous")
                .and_then(|p| p.get("operClass"))
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty());
            let prev_call = v
                .get("previous")
                .and_then(|p| p.get("callsign"))
                .and_then(|x| x.as_str())
                .filter(|s| !s.is_empty());
            let granted = v
                .get("otherInfo")
                .and_then(|o| o.get("grantDate"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let expires = v
                .get("otherInfo")
                .and_then(|o| o.get("expiryDate"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let last_action = v
                .get("otherInfo")
                .and_then(|o| o.get("lastActionDate"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let frn = v
                .get("otherInfo")
                .and_then(|o| o.get("frn"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let ulsuri = v
                .get("otherInfo")
                .and_then(|o| o.get("ulsUrl"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let addr_line1 = v
                .get("address")
                .and_then(|a| a.get("line1"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let addr_line2 = v
                .get("address")
                .and_then(|a| a.get("line2"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let grid = v
                .get("location")
                .and_then(|l| l.get("gridsquare"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let latitude = v
                .get("location")
                .and_then(|l| l.get("latitude"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let longitude = v
                .get("location")
                .and_then(|l| l.get("longitude"))
                .and_then(|x| x.as_str())
                .unwrap_or("");

            let mut out = format!("{r_call} — {name}\n");
            if !status.is_empty() {
                out.push_str(&format!("  status: {status}"));
                if !op_class.is_empty() {
                    out.push_str(&format!(" · class: {op_class}"));
                }
                out.push('\n');
            }
            if let Some(tcall) = trustee_call {
                out.push_str(&format!("  trustee: {tcall}"));
                if let Some(tname) = trustee_name {
                    out.push_str(&format!(" ({tname})"));
                }
                out.push('\n');
            }
            if let (Some(pcall), Some(pclass)) = (prev_call, prev_class) {
                if pcall != r_call || pclass != op_class {
                    out.push_str(&format!("  previous: {pcall} ({pclass})\n"));
                }
            }
            if !granted.is_empty() || !expires.is_empty() || !last_action.is_empty() {
                out.push_str("  dates:");
                if !granted.is_empty() {
                    out.push_str(&format!(" granted {granted}"));
                }
                if !expires.is_empty() {
                    out.push_str(&format!(" · expires {expires}"));
                }
                if !last_action.is_empty() {
                    out.push_str(&format!(" · last action {last_action}"));
                }
                out.push('\n');
            }
            if !addr_line1.is_empty() || !addr_line2.is_empty() {
                let mut addr = String::new();
                if !addr_line1.is_empty() {
                    addr.push_str(addr_line1);
                }
                if !addr_line2.is_empty() {
                    if !addr.is_empty() {
                        addr.push_str(", ");
                    }
                    addr.push_str(addr_line2);
                }
                out.push_str(&format!("  address: {addr}\n"));
            }
            if !grid.is_empty() {
                out.push_str(&format!("  grid: {grid}"));
                if !latitude.is_empty() && !longitude.is_empty() {
                    out.push_str(&format!(" (lat {latitude}, lon {longitude})"));
                }
                out.push('\n');
            }
            if !frn.is_empty() {
                out.push_str(&format!("  FRN: {frn}\n"));
            }
            if !ulsuri.is_empty() {
                out.push_str(&format!("  ULS detail: {ulsuri}\n"));
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "ARRL HQ club station",
                args: r#"{"callsign": "W1AW"}"#,
                note: Some("Returns licensee, class, trustee, grant/expiry, grid square."),
            },
            SkillExample {
                title: "Personal amateur callsign",
                args: r#"{"callsign": "KE8XYZ"}"#,
                note: None,
            },
            SkillExample {
                title: "Non-amateur (GMRS) — friendly redirect",
                args: r#"{"callsign": "WQXY123"}"#,
                note: Some("callook.info is amateur-only; tool returns a ULS web-search link."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Look up the licensee, class, and address behind a US amateur callsign.",
            "Verify a club call's trustee and license status.",
            "Get the grid square / FRN for an amateur operator.",
        ]
    }
}

fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// fcc_amateur_bands — US amateur band plan with per-class privileges
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct BandArgs {
    /// Filter to a single band. Accepts the wavelength label (`"40m"`,
    /// `"2m"`, `"70cm"`), a band name (`"VHF"`, `"HF"`), or a frequency in
    /// MHz (`"14.250"` lands you on 20m). Omit to list every band.
    #[serde(default)]
    band: Option<String>,
    /// Filter to one license class. Accepts `"technician"`, `"general"`,
    /// `"extra"`. Omit to see all class privileges side-by-side.
    #[serde(default)]
    license_class: Option<String>,
}

#[derive(Clone, Copy)]
struct AmateurBand {
    label: &'static str,
    name: &'static str,
    region: &'static str,
    low_mhz: f64,
    high_mhz: f64,
    notes: &'static str,
    /// Per-class summary lines. Empty = no privileges in this class.
    technician: &'static str,
    general: &'static str,
    extra: &'static str,
}

/// US amateur bands per 47 CFR §97.301 / band plan. Source-of-truth references
/// in the description; values current as of 2025. Frequencies in MHz unless
/// noted otherwise.
const BANDS: &[AmateurBand] = &[
    AmateurBand {
        label: "2200m",
        name: "LF (135.7 kHz)",
        region: "LF",
        low_mhz: 0.0001357,
        high_mhz: 0.0001378,
        notes: "1 W EIRP max, CW + narrow-band data only",
        technician: "—",
        general: "Full band, CW + data",
        extra: "Full band, CW + data",
    },
    AmateurBand {
        label: "630m",
        name: "MF (472 kHz)",
        region: "MF",
        low_mhz: 0.000472,
        high_mhz: 0.000479,
        notes: "1-5 W EIRP, CW + narrow-band data",
        technician: "—",
        general: "Full band, CW + data",
        extra: "Full band, CW + data",
    },
    AmateurBand {
        label: "160m",
        name: "Top Band",
        region: "MF",
        low_mhz: 1.800,
        high_mhz: 2.000,
        notes: "Phone and image 1.800-2.000",
        technician: "—",
        general: "Full band CW/phone/data",
        extra: "Full band CW/phone/data",
    },
    AmateurBand {
        label: "80m",
        name: "75/80 meters",
        region: "HF",
        low_mhz: 3.500,
        high_mhz: 4.000,
        notes: "CW 3.500-4.000; phone 3.600-4.000 (segmented by class)",
        technician: "—",
        general: "CW 3.525-3.600, phone 3.800-4.000",
        extra: "CW 3.500-3.600, phone 3.600-4.000 (full)",
    },
    AmateurBand {
        label: "60m",
        name: "60 meters (channelized)",
        region: "HF",
        low_mhz: 5.330,
        high_mhz: 5.405,
        notes: "5 fixed channels: 5332, 5348, 5358.5, 5373, 5405 kHz (USB only, 100W ERP)",
        technician: "—",
        general: "All 5 channels (USB only)",
        extra: "All 5 channels (USB only)",
    },
    AmateurBand {
        label: "40m",
        name: "40 meters",
        region: "HF",
        low_mhz: 7.000,
        high_mhz: 7.300,
        notes: "Phone 7.125-7.300 in Region 2",
        technician: "—",
        general: "CW 7.025-7.125, phone 7.175-7.300",
        extra: "CW 7.000-7.125, phone 7.125-7.300 (full)",
    },
    AmateurBand {
        label: "30m",
        name: "30 meters (WARC)",
        region: "HF",
        low_mhz: 10.100,
        high_mhz: 10.150,
        notes: "CW + narrow-band data only; no phone or image",
        technician: "—",
        general: "Full band, CW + data",
        extra: "Full band, CW + data",
    },
    AmateurBand {
        label: "20m",
        name: "20 meters",
        region: "HF",
        low_mhz: 14.000,
        high_mhz: 14.350,
        notes: "Primary DX band; phone 14.150-14.350 (segmented)",
        technician: "—",
        general: "CW 14.025-14.150, phone 14.225-14.350",
        extra: "CW 14.000-14.150, phone 14.150-14.350 (full)",
    },
    AmateurBand {
        label: "17m",
        name: "17 meters (WARC)",
        region: "HF",
        low_mhz: 18.068,
        high_mhz: 18.168,
        notes: "WARC band; phone 18.110-18.168",
        technician: "—",
        general: "Full band, CW/phone/data",
        extra: "Full band, CW/phone/data",
    },
    AmateurBand {
        label: "15m",
        name: "15 meters",
        region: "HF",
        low_mhz: 21.000,
        high_mhz: 21.450,
        notes: "Strong DX band when 10/12m closed",
        technician: "CW 21.025-21.200",
        general: "CW 21.025-21.200, phone 21.275-21.450",
        extra: "CW 21.000-21.200, phone 21.200-21.450 (full)",
    },
    AmateurBand {
        label: "12m",
        name: "12 meters (WARC)",
        region: "HF",
        low_mhz: 24.890,
        high_mhz: 24.990,
        notes: "WARC band; opens with solar maximum",
        technician: "—",
        general: "Full band, CW/phone/data",
        extra: "Full band, CW/phone/data",
    },
    AmateurBand {
        label: "10m",
        name: "10 meters",
        region: "HF",
        low_mhz: 28.000,
        high_mhz: 29.700,
        notes: "Tech gets HF phone here; FM repeaters 29.50-29.70",
        technician: "CW 28.000-28.300, phone 28.300-28.500 (200W)",
        general: "Full band CW/phone/data",
        extra: "Full band CW/phone/data",
    },
    AmateurBand {
        label: "6m",
        name: "6 meters",
        region: "VHF",
        low_mhz: 50.0,
        high_mhz: 54.0,
        notes: "\"Magic band\" — sporadic-E openings; CW low edge, FM 52-54",
        technician: "Full band",
        general: "Full band",
        extra: "Full band",
    },
    AmateurBand {
        label: "2m",
        name: "2 meters",
        region: "VHF",
        low_mhz: 144.0,
        high_mhz: 148.0,
        notes: "Heaviest VHF activity; FM 144-148 with repeater pairs at ±600 kHz",
        technician: "Full band",
        general: "Full band",
        extra: "Full band",
    },
    AmateurBand {
        label: "1.25m",
        name: "1.25 meters",
        region: "VHF",
        low_mhz: 222.0,
        high_mhz: 225.0,
        notes: "Lightly used; some repeaters",
        technician: "Full band",
        general: "Full band",
        extra: "Full band",
    },
    AmateurBand {
        label: "70cm",
        name: "70 centimeters",
        region: "UHF",
        low_mhz: 420.0,
        high_mhz: 450.0,
        notes: "Heavy repeater + ATV activity; satellites; secondary to gov't radar",
        technician: "Full band",
        general: "Full band",
        extra: "Full band",
    },
    AmateurBand {
        label: "33cm",
        name: "33 centimeters",
        region: "UHF",
        low_mhz: 902.0,
        high_mhz: 928.0,
        notes: "Shared with ISM (Part 15); some FM voice + data",
        technician: "Full band",
        general: "Full band",
        extra: "Full band",
    },
    AmateurBand {
        label: "23cm",
        name: "23 centimeters",
        region: "UHF",
        low_mhz: 1240.0,
        high_mhz: 1300.0,
        notes: "ATV, DATV, weak-signal EME at 1296",
        technician: "Full band",
        general: "Full band",
        extra: "Full band",
    },
    AmateurBand {
        label: "13cm",
        name: "13 centimeters",
        region: "SHF",
        low_mhz: 2300.0,
        high_mhz: 2450.0,
        notes: "Heavily shared with WiFi/ISM (Part 15)",
        technician: "Full band",
        general: "Full band",
        extra: "Full band",
    },
    AmateurBand {
        label: "9cm",
        name: "9 centimeters",
        region: "SHF",
        low_mhz: 3300.0,
        high_mhz: 3500.0,
        notes: "Sunset by 2025 — reallocated to 5G; check current status before operating",
        technician: "Full band (sunset risk)",
        general: "Full band (sunset risk)",
        extra: "Full band (sunset risk)",
    },
    AmateurBand {
        label: "5cm",
        name: "5 centimeters",
        region: "SHF",
        low_mhz: 5650.0,
        high_mhz: 5925.0,
        notes: "Microwave weak-signal + ATV",
        technician: "Full band",
        general: "Full band",
        extra: "Full band",
    },
    AmateurBand {
        label: "3cm",
        name: "3 centimeters",
        region: "SHF",
        low_mhz: 10000.0,
        high_mhz: 10500.0,
        notes: "Popular microwave band; X-band gear",
        technician: "Full band",
        general: "Full band",
        extra: "Full band",
    },
    AmateurBand {
        label: "1.2cm",
        name: "1.25 centimeters",
        region: "EHF",
        low_mhz: 24000.0,
        high_mhz: 24250.0,
        notes: "K-band; experimental",
        technician: "Full band",
        general: "Full band",
        extra: "Full band",
    },
];

fn normalize_class(s: &str) -> Option<&'static str> {
    let l = s.trim().to_ascii_lowercase();
    match l.as_str() {
        "tech" | "technician" | "t" => Some("technician"),
        "gen" | "general" | "g" => Some("general"),
        "extra" | "ae" | "amateur extra" | "e" => Some("extra"),
        _ => None,
    }
}

/// Match a band-filter string to one or more bands.
///   * Empty → every band.
///   * Wavelength label ("40m", "70cm") → exact match on `label`.
///   * Region name (HF, VHF, UHF, SHF, EHF, MF, LF) → all bands in that region.
///   * Numeric MHz → the band whose range contains the value.
fn match_bands(filter: Option<&str>) -> Vec<&'static AmateurBand> {
    let Some(raw) = filter else {
        return BANDS.iter().collect();
    };
    let f = raw.trim();
    if f.is_empty() {
        return BANDS.iter().collect();
    }
    let lower = f.to_ascii_lowercase();
    let region_hit: Vec<&AmateurBand> = BANDS
        .iter()
        .filter(|b| b.region.eq_ignore_ascii_case(f))
        .collect();
    if !region_hit.is_empty() {
        return region_hit;
    }
    if let Ok(mhz) = lower.parse::<f64>() {
        if let Some(b) = BANDS.iter().find(|b| mhz >= b.low_mhz && mhz <= b.high_mhz) {
            return vec![b];
        }
    }
    BANDS
        .iter()
        .filter(|b| b.label.eq_ignore_ascii_case(f))
        .collect()
}

pub struct FccAmateurBands;
impl Skill for FccAmateurBands {
    fn name(&self) -> &'static str {
        "fcc_amateur_bands"
    }
    fn description(&self) -> &'static str {
        "US amateur (ham) radio band plan with per-license-class privileges (Technician / General / \
        Amateur Extra). With no args: list every band from 2200m through 1.25cm with range, region \
        (LF/MF/HF/VHF/UHF/SHF/EHF), per-class summary, and notes. With `band`: filter to one — \
        accepts wavelength label (\"40m\", \"70cm\"), region (\"HF\", \"VHF\"), or a frequency in \
        MHz (\"14.250\" → 20m). With `license_class`: show only that class's privileges."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<BandArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<BandArgs>()?;
            let bands = match_bands(args.band.as_deref());
            if bands.is_empty() {
                let label = args.band.as_deref().unwrap_or("");
                return Ok(text_result(format!(
                    "No US amateur band matches \"{label}\". Try a wavelength label \
                     (40m, 2m, 70cm), a region (HF/VHF/UHF/SHF), or a frequency in MHz."
                )));
            }
            let class = args.license_class.as_deref().and_then(normalize_class);
            let mut out = String::new();
            if let Some(c) = class {
                out.push_str(&format!(
                    "US amateur band plan — privileges for {} class:\n",
                    match c {
                        "technician" => "Technician",
                        "general" => "General",
                        "extra" => "Amateur Extra",
                        _ => c,
                    }
                ));
            } else {
                out.push_str("US amateur band plan (47 CFR §97.301 + bandplan):\n");
            }
            for b in bands {
                out.push_str(&format!("\n  {} ({}) · {}\n", b.label, b.name, b.region));
                let unit = if b.low_mhz < 1.0 { "kHz" } else { "MHz" };
                let scale = if b.low_mhz < 1.0 { 1000.0 } else { 1.0 };
                out.push_str(&format!(
                    "    range: {:.3}–{:.3} {unit}\n",
                    b.low_mhz * scale,
                    b.high_mhz * scale
                ));
                if !b.notes.is_empty() {
                    out.push_str(&format!("    notes: {}\n", b.notes));
                }
                let show_all = class.is_none();
                if show_all || class == Some("technician") {
                    out.push_str(&format!("    Technician: {}\n", b.technician));
                }
                if show_all || class == Some("general") {
                    out.push_str(&format!("    General:    {}\n", b.general));
                }
                if show_all || class == Some("extra") {
                    out.push_str(&format!("    Extra:      {}\n", b.extra));
                }
            }
            out.push_str(
                "\nLegend: \"—\" means no privileges for that class on the band. Power: 1500 W PEP \
                 max output everywhere unless a band-specific note says otherwise (e.g. 60m channels \
                 are 100 W ERP; LF/MF are EIRP-limited).\n",
            );
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Full band plan",
                args: r#"{}"#,
                note: Some("Lists every band 2200m through 1.25cm with all three classes."),
            },
            SkillExample {
                title: "Just 20 meters",
                args: r#"{"band": "20m"}"#,
                note: None,
            },
            SkillExample {
                title: "Frequency lookup",
                args: r#"{"band": "14.250"}"#,
                note: Some("Auto-resolves to the band whose range contains that MHz."),
            },
            SkillExample {
                title: "Technician privileges on HF",
                args: r#"{"band": "HF", "license_class": "technician"}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Look up the frequency range and class privileges of a US amateur band.",
            "Find which band a given frequency lives in.",
            "Quote a Technician/General/Extra's specific sub-band privileges.",
        ]
    }
}

// ---------------------------------------------------------------------------
// fcc_radio_service — FRS / GMRS / MURS / CB and how they relate
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ServiceArgs {
    /// Which non-amateur radio service to describe. Accepts: `"frs"`,
    /// `"gmrs"`, `"murs"`, `"cb"`, or `"compare"` for the side-by-side
    /// table. Omit to list every service plus the compare table.
    #[serde(default)]
    service: Option<String>,
    /// Optional channel number filter (e.g. `7` for FRS channel 7).
    #[serde(default)]
    channel: Option<u32>,
}

struct Channel {
    num: u32,
    freq_mhz: f64,
    /// Power cap in watts (effective radiated). 0 means "see notes".
    power_w: f64,
    note: &'static str,
}

/// 22 FRS channels — license-free, narrowband FM, fixed power caps.
const FRS_CHANNELS: &[Channel] = &[
    Channel {
        num: 1,
        freq_mhz: 462.5625,
        power_w: 2.0,
        note: "shared w/ GMRS",
    },
    Channel {
        num: 2,
        freq_mhz: 462.5875,
        power_w: 2.0,
        note: "shared w/ GMRS",
    },
    Channel {
        num: 3,
        freq_mhz: 462.6125,
        power_w: 2.0,
        note: "shared w/ GMRS",
    },
    Channel {
        num: 4,
        freq_mhz: 462.6375,
        power_w: 2.0,
        note: "shared w/ GMRS",
    },
    Channel {
        num: 5,
        freq_mhz: 462.6625,
        power_w: 2.0,
        note: "shared w/ GMRS",
    },
    Channel {
        num: 6,
        freq_mhz: 462.6875,
        power_w: 2.0,
        note: "shared w/ GMRS",
    },
    Channel {
        num: 7,
        freq_mhz: 462.7125,
        power_w: 2.0,
        note: "shared w/ GMRS",
    },
    Channel {
        num: 8,
        freq_mhz: 467.5625,
        power_w: 0.5,
        note: "FRS only (no GMRS, no repeater)",
    },
    Channel {
        num: 9,
        freq_mhz: 467.5875,
        power_w: 0.5,
        note: "FRS only (no GMRS, no repeater)",
    },
    Channel {
        num: 10,
        freq_mhz: 467.6125,
        power_w: 0.5,
        note: "FRS only (no GMRS, no repeater)",
    },
    Channel {
        num: 11,
        freq_mhz: 467.6375,
        power_w: 0.5,
        note: "FRS only (no GMRS, no repeater)",
    },
    Channel {
        num: 12,
        freq_mhz: 467.6625,
        power_w: 0.5,
        note: "FRS only (no GMRS, no repeater)",
    },
    Channel {
        num: 13,
        freq_mhz: 467.6875,
        power_w: 0.5,
        note: "FRS only (no GMRS, no repeater)",
    },
    Channel {
        num: 14,
        freq_mhz: 467.7125,
        power_w: 0.5,
        note: "FRS only (no GMRS, no repeater)",
    },
    Channel {
        num: 15,
        freq_mhz: 462.5500,
        power_w: 2.0,
        note: "GMRS main; FRS at 2 W only",
    },
    Channel {
        num: 16,
        freq_mhz: 462.5750,
        power_w: 2.0,
        note: "GMRS main; FRS at 2 W only",
    },
    Channel {
        num: 17,
        freq_mhz: 462.6000,
        power_w: 2.0,
        note: "GMRS main; FRS at 2 W only",
    },
    Channel {
        num: 18,
        freq_mhz: 462.6250,
        power_w: 2.0,
        note: "GMRS main; FRS at 2 W only",
    },
    Channel {
        num: 19,
        freq_mhz: 462.6500,
        power_w: 2.0,
        note: "GMRS main; FRS at 2 W only",
    },
    Channel {
        num: 20,
        freq_mhz: 462.6750,
        power_w: 2.0,
        note: "GMRS main; FRS at 2 W only (462.675 = GMRS travel/emergency convention)",
    },
    Channel {
        num: 21,
        freq_mhz: 462.7000,
        power_w: 2.0,
        note: "GMRS main; FRS at 2 W only",
    },
    Channel {
        num: 22,
        freq_mhz: 462.7250,
        power_w: 2.0,
        note: "GMRS main; FRS at 2 W only",
    },
];

/// MURS — 5 VHF channels, license-free, narrowband FM (or wide on 154-x).
const MURS_CHANNELS: &[Channel] = &[
    Channel {
        num: 1,
        freq_mhz: 151.820,
        power_w: 2.0,
        note: "narrowband (11.25 kHz)",
    },
    Channel {
        num: 2,
        freq_mhz: 151.880,
        power_w: 2.0,
        note: "narrowband (11.25 kHz)",
    },
    Channel {
        num: 3,
        freq_mhz: 151.940,
        power_w: 2.0,
        note: "narrowband (11.25 kHz)",
    },
    Channel {
        num: 4,
        freq_mhz: 154.570,
        power_w: 2.0,
        note: "wideband allowed (20 kHz); aka \"Blue Dot\"",
    },
    Channel {
        num: 5,
        freq_mhz: 154.600,
        power_w: 2.0,
        note: "wideband allowed (20 kHz); aka \"Green Dot\"",
    },
];

fn render_frs() -> String {
    let mut out = String::from(
        "FRS — Family Radio Service (license-free, US):\n  \
         License: NONE required (anyone, any age, no FCC fee).\n  \
         Power: 2 W on channels 1-7 and 15-22; 0.5 W on channels 8-14.\n  \
         Bandwidth: 12.5 kHz narrowband FM (8K10F3E).\n  \
         Antenna: integral, non-detachable. No external/gain antennas, no repeaters.\n  \
         CTCSS / DCS \"privacy codes\" are tone-squelch only — they DON'T provide \
         privacy or block other users from sharing the channel.\n  \
         Shares 14 channels with GMRS (1-7, 15-22); FRS is power-capped on those.\n\
        \nChannels:\n",
    );
    for c in FRS_CHANNELS {
        out.push_str(&format!(
            "  ch{:>2}: {:.4} MHz · {} W · {}\n",
            c.num, c.freq_mhz, c.power_w, c.note
        ));
    }
    out
}

fn render_gmrs() -> String {
    let mut out = String::from(
        "GMRS — General Mobile Radio Service (US, licensed):\n  \
         License: REQUIRED — single $35 GMRS license (FCC form 605) covers the licensee \
         and their immediate family for 10 years. No exam.\n  \
         Power: up to 50 W on main channels (1-7, 15-22) and on GMRS repeater inputs \
         (467.550-467.725 = main channels +5 MHz offset, channels labeled \"15R-22R\" or \
         \"RPT15-RPT22\"). 5 W on interstitial channels (8-14, which GMRS shares with FRS).\n  \
         Bandwidth: 20 kHz (wideband) on main channels, 12.5 kHz (narrowband) on \
         interstitial. Most GMRS gear is 16K0F3E.\n  \
         Antenna: any (including base/mobile/gain).\n  \
         Modes: FM voice, plus data per FCC waivers (digital + tone-squelch + APRS-like \
         beaconing is increasingly permitted).\n  \
         Repeaters: YES — GMRS repeaters use 467.xxx (input) → 462.xxx (output) with the \
         +5 MHz offset. Common community / club resource.\n  \
         Sharing with FRS: GMRS users hear and can be heard by FRS users on the shared \
         14 channels; mind the FRS power caps and the no-repeater rule on 8-14.\n\
        \nMain channels (shared with FRS 1-7 + 15-22): see fcc_radio_service service=\"frs\".\n  \
         Repeater inputs (GMRS-only): 467.5500, 467.5750, 467.6000, 467.6250, 467.6500, 467.6750, \
         467.7000, 467.7250 MHz (paired with output = input − 5 MHz).\n",
    );
    let _ = &mut out;
    out
}

fn render_murs() -> String {
    let mut out = String::from(
        "MURS — Multi-Use Radio Service (US, license-free):\n  \
         License: NONE required.\n  \
         Power: 2 W ERP max on all 5 VHF channels.\n  \
         Bandwidth: 11.25 kHz (narrowband) on channels 1-3, 20 kHz (wideband) on 4-5.\n  \
         Antenna: external antennas allowed (unlike FRS); height limit 60 ft above ground / \
         20 ft above the structure it's mounted on, whichever is greater.\n  \
         Modes: FM voice, narrowband data, and tone-squelch. No repeaters allowed.\n  \
         Use: business/personal/farm; common with wildlife cameras (\"Dakota Alert\" beam \
         break sensors) and small-property comms. Less crowded than FRS/GMRS.\n\
        \nChannels:\n",
    );
    for c in MURS_CHANNELS {
        out.push_str(&format!(
            "  ch{}: {:.3} MHz · {} W · {}\n",
            c.num, c.freq_mhz, c.power_w, c.note
        ));
    }
    out
}

fn render_cb() -> String {
    String::from(
        "CB — Citizens Band Radio Service (US, license-free):\n  \
         License: NONE required.\n  \
         40 channels in the 11-meter HF band (26.965-27.405 MHz).\n  \
         Power: 4 W carrier (AM) / 12 W PEP (SSB) max output.\n  \
         Bandwidth: 8 kHz AM or SSB. Modes: AM (most common), SSB (USB/LSB), some FM since 2021.\n  \
         Antenna: external mobile or base, any reasonable size; tuning matters (HF antennas are big).\n  \
         No repeaters, no encryption, no \"phone patch\" allowed.\n\
        \nNotable channels:\n  \
         ch9  = 27.065 MHz (emergency / motorist assist)\n  \
         ch19 = 27.185 MHz (truckers, road conditions, the de-facto highway channel)\n  \
         ch11 = 27.085 MHz (calling channel, AM)\n  \
         ch36 = 27.365 MHz (typical LSB calling channel, SSB conversation common above ch28)\n",
    )
}

fn render_compare() -> String {
    String::from(
        "Comparison of US license-free / low-license-bar voice radio services:\n\
         \n  Service  Band      Channels  Power            License?       Repeaters?  Antenna?\n  \
         FRS      UHF       22        0.5-2 W ERP      NO             NO          integral only\n  \
         GMRS     UHF       30        up to 50 W ERP   YES ($35/10y)  YES         any\n  \
         MURS     VHF       5         2 W ERP          NO             NO          external OK (height-limited)\n  \
         CB       HF (11m)  40        4 W AM / 12 W SSB NO            NO          external OK\n  \
         Amateur  HF/V/UHF  many      1500 W PEP       YES (exam)     YES         any\n\
         \nFRS ↔ GMRS share 14 channels (1-7, 15-22). When a GMRS-licensed and FRS-only user are \
         on the same channel they can hear each other — the GMRS user just has more power, \
         wideband on the main channels, and repeater access available.\n\
         FRS specifically has NO callsigns (it's unlicensed); GMRS and amateur do (look up via \
         fcc_callsign). MURS and CB don't issue callsigns either.\n",
    )
}

pub struct FccRadioService;
impl Skill for FccRadioService {
    fn name(&self) -> &'static str {
        "fcc_radio_service"
    }
    fn description(&self) -> &'static str {
        "Reference data for US non-amateur personal radio services: FRS (Family Radio Service), \
        GMRS (General Mobile Radio Service), MURS (Multi-Use Radio Service), and CB (Citizens \
        Band). Returns license requirements, channel maps with frequencies, power caps, antenna \
        rules, and how the services share spectrum (notably FRS↔GMRS). Use `service=\"compare\"` \
        for the side-by-side; `channel=N` filters to one channel for FRS / MURS / CB. \
        Note: only GMRS and amateur issue callsigns — FRS / MURS / CB are unlicensed."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ServiceArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_server, args) = ctx.parse::<ServiceArgs>()?;
            let svc = args
                .service
                .as_deref()
                .map(str::trim)
                .map(str::to_ascii_lowercase);
            let body = match svc.as_deref() {
                None => {
                    // No filter: full dump.
                    let mut out = render_compare();
                    out.push_str("\n────────────────\n\n");
                    out.push_str(&render_frs());
                    out.push_str("\n────────────────\n\n");
                    out.push_str(&render_gmrs());
                    out.push_str("\n────────────────\n\n");
                    out.push_str(&render_murs());
                    out.push_str("\n────────────────\n\n");
                    out.push_str(&render_cb());
                    out
                }
                Some("compare") | Some("all") => render_compare(),
                Some("frs") => filter_channel(render_frs, FRS_CHANNELS, args.channel),
                Some("murs") => filter_channel(render_murs, MURS_CHANNELS, args.channel),
                Some("gmrs") => render_gmrs(),
                Some("cb") => render_cb(),
                Some(other) => {
                    return Err(invalid(format!(
                        "unknown service \"{other}\". Pick one of: frs, gmrs, murs, cb, compare"
                    )))
                }
            };
            Ok(text_result(body))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Side-by-side comparison",
                args: r#"{"service": "compare"}"#,
                note: Some("Quick table of FRS / GMRS / MURS / CB / amateur."),
            },
            SkillExample {
                title: "GMRS overview",
                args: r#"{"service": "gmrs"}"#,
                note: Some("Power, license cost, repeater rules, channel sharing with FRS."),
            },
            SkillExample {
                title: "Single FRS channel",
                args: r#"{"service": "frs", "channel": 7}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Answer 'do I need a license for X?' for US personal-radio services.",
            "Look up the frequency and power cap of a specific FRS / MURS / CB channel.",
            "Explain how FRS and GMRS share spectrum on the 462/467 MHz channels.",
        ]
    }
}

/// Helper: when a `channel` filter is set, render only that channel + the
/// service preamble. When it isn't, render the full body.
fn filter_channel(full: fn() -> String, channels: &'static [Channel], ch: Option<u32>) -> String {
    let Some(num) = ch else {
        return full();
    };
    match channels.iter().find(|c| c.num == num) {
        Some(c) => format!(
            "Channel {}: {:.4} MHz · {} W · {}\n",
            c.num, c.freq_mhz, c.power_w, c.note
        ),
        None => format!(
            "No channel {num} in this service. Valid: {}.",
            channels
                .iter()
                .map(|c| c.num.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(FccCallsign),
        Box::new(FccAmateurBands),
        Box::new(FccRadioService),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_bands_by_label() {
        let r = match_bands(Some("40m"));
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].label, "40m");
    }

    #[test]
    fn match_bands_by_frequency() {
        let r = match_bands(Some("14.250"));
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].label, "20m");
    }

    #[test]
    fn match_bands_by_region() {
        let r = match_bands(Some("HF"));
        assert!(r.iter().any(|b| b.label == "20m"));
        assert!(r.iter().any(|b| b.label == "40m"));
        assert!(!r.iter().any(|b| b.label == "2m"));
    }

    #[test]
    fn match_bands_unknown_returns_empty() {
        assert!(match_bands(Some("xxx")).is_empty());
    }

    #[test]
    fn normalize_class_accepts_aliases() {
        assert_eq!(normalize_class("Technician"), Some("technician"));
        assert_eq!(normalize_class("tech"), Some("technician"));
        assert_eq!(normalize_class("Gen"), Some("general"));
        assert_eq!(normalize_class("Amateur Extra"), Some("extra"));
        assert_eq!(normalize_class("nope"), None);
    }

    #[test]
    fn frs_channels_cover_22_with_correct_power() {
        assert_eq!(FRS_CHANNELS.len(), 22);
        // Channels 8-14 are 0.5 W; everything else 2 W.
        for c in FRS_CHANNELS {
            if (8..=14).contains(&c.num) {
                assert!(
                    (c.power_w - 0.5).abs() < 1e-6,
                    "ch{} should be 0.5 W",
                    c.num
                );
            } else {
                assert!(
                    (c.power_w - 2.0).abs() < 1e-6,
                    "ch{} should be 2.0 W",
                    c.num
                );
            }
        }
    }

    /// Live: hit callook.info for W1AW (ARRL HQ club station, perpetual). The
    /// test confirms the wire shape we parse against. Opt-in via
    /// `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn fcc_callsign_w1aw_live() {
        let http = reqwest::Client::builder()
            .user_agent(crate::LODESTONE_UA)
            .build()
            .unwrap();
        let v: serde_json::Value = http
            .get("https://callook.info/W1AW/json")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(
            v.get("status").and_then(|x| x.as_str()),
            Some("VALID"),
            "W1AW should always come back as VALID"
        );
        // Fields we parse against.
        assert!(v.pointer("/current/callsign").is_some());
        assert!(v.pointer("/current/operClass").is_some());
        assert!(v.pointer("/otherInfo/grantDate").is_some());
        assert!(v.pointer("/otherInfo/frn").is_some());
    }
}
