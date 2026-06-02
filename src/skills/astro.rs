//! Astronomy skills — pure-formula Sun and Moon position/rise-set, plus a
//! small bright-named-star catalog with topocentric alt/az from an observer.
//! All computation is local (no network); algorithms are abridged from Meeus
//! ("Astronomical Algorithms", 2nd ed.). Off by default (`[astro].enabled`).

use std::sync::Arc;

use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone, Timelike, Utc};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{invalid, text_result};

fn deg(r: f64) -> f64 {
    r.to_degrees()
}
fn rad(d: f64) -> f64 {
    d.to_radians()
}
fn norm360(x: f64) -> f64 {
    x.rem_euclid(360.0)
}

fn julian_day(dt: &NaiveDateTime) -> f64 {
    let (mut y, mut m) = (dt.year() as f64, dt.month() as f64);
    if dt.month() <= 2 {
        y -= 1.0;
        m += 12.0;
    }
    let a = (y / 100.0).floor();
    let b = 2.0 - a + (a / 4.0).floor();
    let day_frac =
        (dt.hour() as f64 + dt.minute() as f64 / 60.0 + dt.second() as f64 / 3600.0) / 24.0;
    (365.25 * (y + 4716.0)).floor() + (30.6001 * (m + 1.0)).floor() + dt.day() as f64 + b - 1524.5
        + day_frac
}

/// Greenwich Mean Sidereal Time in degrees (IAU 1982).
fn gmst_deg(dt: &NaiveDateTime) -> f64 {
    let jd = julian_day(dt);
    let t = (jd - 2451545.0) / 36525.0;
    norm360(
        280.46061837 + 360.98564736629 * (jd - 2451545.0) + 0.000387933 * t * t
            - t * t * t / 38_710_000.0,
    )
}

/// Convert equatorial (RA, Dec in degrees) to topocentric (alt, az in degrees).
fn equ_to_topo(
    ra_deg: f64,
    dec_deg: f64,
    lat_deg: f64,
    lon_deg: f64,
    dt: &NaiveDateTime,
) -> (f64, f64) {
    let lst = norm360(gmst_deg(dt) + lon_deg);
    let ha = rad(norm360(lst - ra_deg));
    let dec = rad(dec_deg);
    let lat = rad(lat_deg);
    let alt = (lat.sin() * dec.sin() + lat.cos() * dec.cos() * ha.cos()).asin();
    // North-referenced azimuth (0° = N, 90° = E, 180° = S, 270° = W) so it can
    // be passed straight to `compass()`. Meeus 13.6 gives the south-referenced
    // angle; we add 180° to rebase to north so a body due south reports 180°,
    // not 0°.
    let az = (-ha.sin()).atan2(lat.cos() * dec.tan() - lat.sin() * ha.cos());
    (deg(alt), norm360(deg(az) + 180.0))
}

/// Sun position (RA, Dec in degrees) at `dt` — Meeus low-accuracy.
fn sun_radec(dt: &NaiveDateTime) -> (f64, f64) {
    let jd = julian_day(dt);
    let n = jd - 2451545.0;
    let l = norm360(280.460 + 0.9856474 * n);
    let g = rad(norm360(357.528 + 0.9856003 * n));
    let lambda = rad(l + 1.915 * g.sin() + 0.020 * (2.0 * g).sin());
    let eps = rad(23.439 - 0.0000004 * n);
    let ra = lambda.sin().atan2(lambda.cos() * eps.cos()).to_degrees();
    let dec = (eps.sin() * lambda.sin()).asin().to_degrees();
    (norm360(ra), dec)
}

/// Moon position (RA, Dec in degrees) at `dt` — heavily abridged Meeus.
fn moon_radec(dt: &NaiveDateTime) -> (f64, f64) {
    let jd = julian_day(dt);
    let t = (jd - 2451545.0) / 36525.0;
    let lp = norm360(218.3164477 + 481267.88123421 * t);
    let d = norm360(297.8501921 + 445267.1114034 * t);
    let m = norm360(357.5291092 + 35999.0502909 * t);
    let mp = norm360(134.9633964 + 477198.8675055 * t);
    let f = norm360(93.2720950 + 483202.0175233 * t);
    let lambda = lp + 6.289 * rad(mp).sin() - 1.274 * rad(2.0 * d - mp).sin()
        + 0.658 * rad(2.0 * d).sin()
        - 0.186 * rad(m).sin()
        - 0.059 * rad(2.0 * mp - 2.0 * d).sin();
    // Top three terms of Meeus Table 47.B (ecliptic latitude). The third term
    // is +0.173·sin(M′ − F) — earlier code used 0.278 which is the coefficient
    // from a different table and caused a ~0.1° declination error.
    let beta = 5.128 * rad(f).sin() + 0.281 * rad(mp + f).sin() + 0.173 * rad(mp - f).sin();
    let eps = rad(23.439 - 0.0000004 * (jd - 2451545.0));
    let lam = rad(lambda);
    let bet = rad(beta);
    let ra = (lam.sin() * eps.cos() - bet.tan() * eps.sin()).atan2(lam.cos());
    let dec = (bet.sin() * eps.cos() + bet.cos() * eps.sin() * lam.sin()).asin();
    (norm360(deg(ra)), deg(dec))
}

/// Approximate Sun-Moon elongation → phase + illumination%.
fn moon_phase(dt: &NaiveDateTime) -> (f64, &'static str) {
    let (s_ra, s_dec) = sun_radec(dt);
    let (m_ra, m_dec) = moon_radec(dt);
    let s = rad(s_ra);
    let m = rad(m_ra);
    let cos_psi =
        rad(s_dec).sin() * rad(m_dec).sin() + rad(s_dec).cos() * rad(m_dec).cos() * (m - s).cos();
    let psi = cos_psi.clamp(-1.0, 1.0).acos();
    let illumination = 0.5 * (1.0 - psi.cos()) * 100.0;
    let phase = norm360(deg(m - s));
    let name = if !(22.5..337.5).contains(&phase) {
        "new"
    } else if phase < 67.5 {
        "waxing crescent"
    } else if phase < 112.5 {
        "first quarter"
    } else if phase < 157.5 {
        "waxing gibbous"
    } else if phase < 202.5 {
        "full"
    } else if phase < 247.5 {
        "waning gibbous"
    } else if phase < 292.5 {
        "last quarter"
    } else {
        "waning crescent"
    };
    (illumination, name)
}

/// Scan a day for sunrise/sunset (geometric horizon at -0.833° accounting for refraction).
fn rise_set(
    radec: impl Fn(&NaiveDateTime) -> (f64, f64),
    refraction_deg: f64,
    lat: f64,
    lon: f64,
    date: chrono::NaiveDate,
) -> (
    Option<NaiveDateTime>,
    Option<NaiveDateTime>,
    Option<NaiveDateTime>,
) {
    let start = date.and_hms_opt(0, 0, 0).unwrap();
    let step = chrono::Duration::seconds(60);
    let mut prev = start;
    let (ra0, dec0) = radec(&prev);
    let (mut prev_alt, _) = equ_to_topo(ra0, dec0, lat, lon, &prev);
    let mut rise = None;
    let mut set = None;
    let mut transit = None;
    let mut peak_alt = prev_alt;
    let mut peak_t = prev;
    for _ in 0..1440 {
        let t = prev + step;
        let (ra, dec) = radec(&t);
        let (alt, _) = equ_to_topo(ra, dec, lat, lon, &t);
        let thr = -refraction_deg;
        if rise.is_none() && prev_alt < thr && alt >= thr {
            rise = Some(t);
        }
        if set.is_none() && prev_alt >= thr && alt < thr {
            set = Some(t);
        }
        if alt > peak_alt {
            peak_alt = alt;
            peak_t = t;
        }
        prev_alt = alt;
        prev = t;
    }
    if peak_alt > -refraction_deg {
        transit = Some(peak_t);
    }
    (rise, transit, set)
}

fn parse_time(s: Option<&str>) -> Result<NaiveDateTime, McpError> {
    let Some(s) = s.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Utc::now().naive_utc());
    };
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.naive_utc());
    }
    for fmt in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d"] {
        if let Ok(d) = NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(d);
        }
        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return Ok(d.and_hms_opt(0, 0, 0).unwrap());
        }
    }
    Err(invalid(format!("could not parse time '{s}' (use RFC3339)")))
}

fn fmt_local_or_utc(d: Option<NaiveDateTime>) -> String {
    match d {
        Some(d) => Utc.from_utc_datetime(&d).format("%H:%M UTC").to_string(),
        None => "—".to_string(),
    }
}

fn compass(az: f64) -> &'static str {
    const P: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    let i = (((norm360(az) / 22.5) + 0.5).floor() as usize) % 16;
    P[i]
}

// --- arg structs ---

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ObsTimeArgs {
    /// Observer latitude (decimal degrees).
    lat: f64,
    /// Observer longitude (decimal degrees).
    lon: f64,
    /// Time as RFC3339; omit for now.
    #[serde(default)]
    at: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StarArgs {
    /// Star name (case-insensitive). See astro_star_list for the catalog.
    name: String,
    /// Observer latitude (decimal degrees).
    lat: f64,
    /// Observer longitude (decimal degrees).
    lon: f64,
    /// Time as RFC3339; omit for now.
    #[serde(default)]
    at: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct StarListArgs {
    /// Optional name substring filter.
    #[serde(default)]
    filter: Option<String>,
}

// --- skills ---

pub struct AstroSun;
impl Skill for AstroSun {
    fn name(&self) -> &'static str {
        "astro_sun"
    }
    fn description(&self) -> &'static str {
        "Sun: current altitude/azimuth from an observer + today's sunrise / transit (solar noon) \
        / sunset (all UTC). Pure formula (Meeus low-accuracy)."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ObsTimeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<ObsTimeArgs>()?;
            let when = parse_time(args.at.as_deref())?;
            let (ra, dec) = sun_radec(&when);
            let (alt, az) = equ_to_topo(ra, dec, args.lat, args.lon, &when);
            let (rise, transit, set) = rise_set(sun_radec, 0.833, args.lat, args.lon, when.date());
            Ok(text_result(format!(
                "Sun at {} from ({:.4}, {:.4}):\n  altitude: {:>5.1}°  azimuth: {:>5.1}° ({})\n  RA/Dec: {:.2}°, {:.2}°\nToday: rise {} · transit {} · set {}",
                when.format("%Y-%m-%d %H:%M UTC"),
                args.lat,
                args.lon,
                alt,
                az,
                compass(az),
                ra,
                dec,
                fmt_local_or_utc(rise),
                fmt_local_or_utc(transit),
                fmt_local_or_utc(set),
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Right now from Seattle",
                args: r#"{"lat": 47.6062, "lon": -122.3321}"#,
                note: Some("Returns current alt/az plus today's rise / transit / set in UTC."),
            },
            SkillExample {
                title: "Specific UTC instant at the Tropic of Cancer",
                args: r#"{"lat": 23.43, "lon": 0.0, "at": "2026-06-21T12:00:00Z"}"#,
                note: Some(
                    "June solstice noon (Sun declination ≈ +23.44°); altitude ≈ 90° here, \
                     ≈ 66.6° at the equator.",
                ),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Check the Sun's current altitude / azimuth from a known observer.",
            "Get today's sunrise / solar noon / sunset times in UTC.",
            "Plan an outdoor activity around when the Sun will be above the horizon.",
        ]
    }
}

pub struct AstroMoon;
impl Skill for AstroMoon {
    fn name(&self) -> &'static str {
        "astro_moon"
    }
    fn description(&self) -> &'static str {
        "Moon: current altitude/azimuth from an observer, illumination %, phase name, and \
        today's moonrise / transit / moonset (UTC). Abridged Meeus."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ObsTimeArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<ObsTimeArgs>()?;
            let when = parse_time(args.at.as_deref())?;
            let (ra, dec) = moon_radec(&when);
            let (alt, az) = equ_to_topo(ra, dec, args.lat, args.lon, &when);
            let (illum, phase) = moon_phase(&when);
            // Moon rise/set: standard altitude h₀ ≈ +0.125° (refraction
            // ≈ +0.567° minus mean lunar parallax ≈ −0.95° plus mean
            // semi-diameter ≈ +0.26°), per Meeus §15. The earlier value
            // 0.567° was refraction only and placed rise/set several
            // minutes early/late.
            let (rise, transit, set) = rise_set(moon_radec, 0.125, args.lat, args.lon, when.date());
            Ok(text_result(format!(
                "Moon at {} from ({:.4}, {:.4}):\n  altitude: {:>5.1}°  azimuth: {:>5.1}° ({})\n  illumination: {:.0}%  phase: {}\n  RA/Dec: {:.2}°, {:.2}°\nToday: rise {} · transit {} · set {}",
                when.format("%Y-%m-%d %H:%M UTC"),
                args.lat,
                args.lon,
                alt,
                az,
                compass(az),
                illum,
                phase,
                ra,
                dec,
                fmt_local_or_utc(rise),
                fmt_local_or_utc(transit),
                fmt_local_or_utc(set),
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Tonight's moon from London",
                args: r#"{"lat": 51.5074, "lon": -0.1278}"#,
                note: Some("Reports illumination %, phase name, and today's rise / transit / set."),
            },
            SkillExample {
                title: "Specific UTC instant",
                args: r#"{"lat": -33.8688, "lon": 151.2093, "at": "2026-06-15T10:00:00Z"}"#,
                note: Some("Sydney; check whether the Moon is up at this UTC time."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Get the Moon's current alt/az and phase from a known observer.",
            "Check tonight's moonrise / moonset and illumination percentage.",
            "Plan astrophotography around when the Moon won't wash out the sky.",
        ]
    }
}

/// Bright named-star catalog. RA in degrees (J2000), Dec in degrees, apparent V magnitude.
const STARS: &[(&str, f64, f64, f64, &str)] = &[
    ("Sirius", 101.2872, -16.7161, -1.46, "alpha CMa"),
    ("Canopus", 95.9879, -52.6957, -0.74, "alpha Car"),
    ("Arcturus", 213.9154, 19.1825, -0.05, "alpha Boo"),
    ("Vega", 279.2347, 38.7837, 0.03, "alpha Lyr"),
    ("Capella", 79.1723, 45.9980, 0.08, "alpha Aur"),
    ("Rigel", 78.6345, -8.2017, 0.13, "beta Ori"),
    ("Procyon", 114.8254, 5.2250, 0.34, "alpha CMi"),
    ("Achernar", 24.4285, -57.2367, 0.46, "alpha Eri"),
    ("Betelgeuse", 88.7929, 7.4070, 0.50, "alpha Ori"),
    ("Hadar", 210.9559, -60.3730, 0.61, "beta Cen"),
    ("Altair", 297.6958, 8.8683, 0.77, "alpha Aql"),
    ("Aldebaran", 68.9802, 16.5093, 0.85, "alpha Tau"),
    ("Acrux", 186.6496, -63.0991, 0.77, "alpha Cru"),
    ("Antares", 247.3519, -26.4320, 1.09, "alpha Sco"),
    ("Spica", 201.2983, -11.1614, 1.04, "alpha Vir"),
    ("Pollux", 116.3289, 28.0262, 1.14, "beta Gem"),
    ("Fomalhaut", 344.4127, -29.6222, 1.16, "alpha PsA"),
    ("Deneb", 310.3580, 45.2803, 1.25, "alpha Cyg"),
    ("Mimosa", 191.9303, -59.6889, 1.30, "beta Cru"),
    ("Regulus", 152.0930, 11.9672, 1.40, "alpha Leo"),
    ("Adhara", 104.6564, -28.9721, 1.50, "epsilon CMa"),
    ("Castor", 113.6499, 31.8883, 1.58, "alpha Gem"),
    ("Gacrux", 187.7915, -57.1131, 1.63, "gamma Cru"),
    ("Shaula", 263.4022, -37.1038, 1.62, "lambda Sco"),
    ("Bellatrix", 81.2828, 6.3497, 1.64, "gamma Ori"),
    ("Elnath", 81.5728, 28.6075, 1.65, "beta Tau"),
    ("Miaplacidus", 138.2999, -69.7172, 1.67, "beta Car"),
    ("Alnilam", 84.0533, -1.2019, 1.69, "epsilon Ori"),
    ("Alnitak", 85.1897, -1.9426, 1.74, "zeta Ori"),
    ("Polaris", 37.9546, 89.2641, 1.97, "alpha UMi (North Star)"),
    ("Dubhe", 165.9320, 61.7510, 1.79, "alpha UMa"),
    ("Mirfak", 51.0808, 49.8612, 1.79, "alpha Per"),
    ("Wezen", 107.0980, -26.3933, 1.83, "delta CMa"),
    ("Mizar", 200.9814, 54.9254, 2.04, "zeta UMa"),
    ("Algol", 47.0422, 40.9556, 2.12, "beta Per"),
    ("Alphard", 141.8968, -8.6587, 1.99, "alpha Hya"),
    ("Hamal", 31.7935, 23.4625, 2.00, "alpha Ari"),
];

fn find_star(name: &str) -> Option<&'static (&'static str, f64, f64, f64, &'static str)> {
    let lc = name.trim().to_ascii_lowercase();
    STARS.iter().find(|(n, ..)| n.to_ascii_lowercase() == lc)
}

pub struct AstroStar;
impl Skill for AstroStar {
    fn name(&self) -> &'static str {
        "astro_star"
    }
    fn description(&self) -> &'static str {
        "Topocentric altitude and azimuth of a NAMED bright star (built-in catalog of ~35) from \
        an observer at `at` (UTC; omit for now). Use astro_star_list to see what's catalogued."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StarArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<StarArgs>()?;
            let star = find_star(&args.name).ok_or_else(|| {
                invalid(format!(
                    "no catalogued star named \"{}\" — try astro_star_list",
                    args.name
                ))
            })?;
            let when = parse_time(args.at.as_deref())?;
            let (alt, az) = equ_to_topo(star.1, star.2, args.lat, args.lon, &when);
            let above = if alt > 0.0 {
                "above horizon"
            } else {
                "below horizon"
            };
            Ok(text_result(format!(
                "{} ({}) at {} from ({:.4}, {:.4}):\n  altitude: {:>5.1}°  azimuth: {:>5.1}° ({})\n  RA/Dec (J2000): {:.2}°, {:.2}°  V mag: {:.2}\n  {}",
                star.0,
                star.4,
                when.format("%Y-%m-%d %H:%M UTC"),
                args.lat,
                args.lon,
                alt,
                az,
                compass(az),
                star.1,
                star.2,
                star.3,
                above
            )))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Polaris from New York",
                args: r#"{"name": "Polaris", "lat": 40.7128, "lon": -74.0060}"#,
                note: Some("Altitude should be close to the observer's latitude."),
            },
            SkillExample {
                title: "Sirius at a specific time",
                args: r#"{"name": "Sirius", "lat": 34.0522, "lon": -118.2437, "at": "2026-12-15T08:00:00Z"}"#,
                note: Some("Use `astro_star_list` if the name isn't in the catalog."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Point a telescope at a named bright star from a known observer.",
            "Check whether a specific cataloged star is currently above the horizon.",
            "Label a sky direction with the nearest bright star.",
        ]
    }
}

pub struct AstroStarList;
impl Skill for AstroStarList {
    fn name(&self) -> &'static str {
        "astro_star_list"
    }
    fn description(&self) -> &'static str {
        "List the built-in bright-star catalog (name, Bayer designation, RA/Dec J2000, V mag). \
        Optional `filter` substring."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<StarListArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<StarListArgs>()?;
            let needle = args
                .filter
                .as_ref()
                .map(|s| s.to_ascii_lowercase())
                .filter(|s| !s.is_empty());
            let mut out = format!("{} bright stars catalogued:\n", STARS.len());
            for (name, ra, dec, mag, desig) in STARS {
                if needle.as_ref().is_some_and(|n| {
                    !name.to_ascii_lowercase().contains(n)
                        && !desig.to_ascii_lowercase().contains(n)
                }) {
                    continue;
                }
                out.push_str(&format!(
                    "  {:<12}  {}  RA {:>7.2}°  Dec {:>7.2}°  V {:>5.2}\n",
                    name, desig, ra, dec, mag
                ));
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Whole catalog",
                args: r#"{}"#,
                note: Some("Lists every catalogued star with Bayer designation and J2000 coords."),
            },
            SkillExample {
                title: "Filter by Orion designation",
                args: r#"{"filter": "Ori"}"#,
                note: Some("Substring match against name and Bayer designation."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Discover which star names are accepted by `astro_star`.",
            "Find catalog entries belonging to a constellation by Bayer designation.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct VisibleArgs {
    /// Observer latitude (decimal degrees).
    lat: f64,
    /// Observer longitude (decimal degrees).
    lon: f64,
    /// Minimum altitude above the horizon, degrees (default 10°).
    #[serde(default)]
    min_altitude_deg: Option<f64>,
    /// Optional V-magnitude limit (only stars brighter than this; default 2.5).
    #[serde(default)]
    max_magnitude: Option<f64>,
    /// Time as RFC3339; omit for now.
    #[serde(default)]
    at: Option<String>,
}

pub struct AstroVisibleStars;
impl Skill for AstroVisibleStars {
    fn name(&self) -> &'static str {
        "astro_visible_stars"
    }
    fn description(&self) -> &'static str {
        "Of the built-in bright catalog, list the stars currently above the horizon (≥ \
        `min_altitude_deg`, default 10°) from an observer, optionally filtered by V magnitude. \
        Sorted by altitude (highest first) so you know what's overhead RIGHT NOW. SIRIL-style \
        'what can I see' for naked-eye targets."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<VisibleArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<VisibleArgs>()?;
            let when = parse_time(args.at.as_deref())?;
            let min_alt = args.min_altitude_deg.unwrap_or(10.0);
            let max_mag = args.max_magnitude.unwrap_or(2.5);
            let mut hits: Vec<(&'static str, f64, f64, f64, f64, &'static str)> = STARS
                .iter()
                .filter(|(_, _, _, mag, _)| *mag <= max_mag)
                .filter_map(|(name, ra, dec, mag, desig)| {
                    let (alt, az) = equ_to_topo(*ra, *dec, args.lat, args.lon, &when);
                    if alt >= min_alt {
                        Some((*name, alt, az, *mag, *ra, *desig))
                    } else {
                        None
                    }
                })
                .collect();
            hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            if hits.is_empty() {
                return Ok(text_result(format!(
                    "No catalog star is above {:.0}° (V ≤ {:.1}) at {} from ({:.4}, {:.4}).",
                    min_alt,
                    max_mag,
                    when.format("%Y-%m-%d %H:%M UTC"),
                    args.lat,
                    args.lon
                )));
            }
            let mut out = format!(
                "{} bright star(s) above {:.0}° (V ≤ {:.1}) at {} from ({:.4}, {:.4}):\n  star          alt    az          V mag  designation\n",
                hits.len(),
                min_alt,
                max_mag,
                when.format("%Y-%m-%d %H:%M UTC"),
                args.lat,
                args.lon
            );
            for (name, alt, az, mag, _ra, desig) in &hits {
                out.push_str(&format!(
                    "  {:<12}  {:>5.1}°  {:>5.1}° {:<3}  {:>5.2}  {}\n",
                    name,
                    alt,
                    az,
                    compass(*az),
                    mag,
                    desig
                ));
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "What's overhead now",
                args: r#"{"lat": 47.6062, "lon": -122.3321}"#,
                note: Some("Bright stars (V ≤ 2.5) at least 10° above the horizon, sorted by altitude (highest first)."),
            },
            SkillExample {
                title: "Loosen the limits",
                args: r#"{"lat": 47.6062, "lon": -122.3321, "min_altitude_deg": 20, "max_magnitude": 1.5}"#,
                note: Some("Only the very brightest, well clear of the horizon."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Find naked-eye targets currently above the horizon for a stargazing session.",
            "Filter the bright-star catalog by altitude and magnitude in one call.",
            "Build a quick 'what can I see right now' list for an observer.",
        ]
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct IdentifyArgs {
    /// Right Ascension in degrees (J2000).
    ra_deg: f64,
    /// Declination in degrees (J2000).
    dec_deg: f64,
    /// Max angular distance in degrees to consider a match (default 5°, capped at 30).
    #[serde(default)]
    tolerance_deg: Option<f64>,
    /// Max candidates to return (default 5, capped at 20).
    #[serde(default)]
    max: Option<u32>,
}

/// Great-circle angular distance between two equatorial points in degrees.
fn angular_sep(ra1: f64, dec1: f64, ra2: f64, dec2: f64) -> f64 {
    let d = (rad(dec1).sin() * rad(dec2).sin()
        + rad(dec1).cos() * rad(dec2).cos() * rad(ra1 - ra2).cos())
    .clamp(-1.0, 1.0);
    deg(d.acos())
}

pub struct AstroIdentify;
impl Skill for AstroIdentify {
    fn name(&self) -> &'static str {
        "astro_identify"
    }
    fn description(&self) -> &'static str {
        "Identify catalog star(s) near a given sky direction (RA, Dec in J2000 degrees). Returns \
        matches within `tolerance_deg` (default 5°), nearest first. Useful for plate-solving \
        debugging or labeling what you're pointing at."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<IdentifyArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (_, args) = ctx.parse::<IdentifyArgs>()?;
            let tol = args.tolerance_deg.unwrap_or(5.0).clamp(0.01, 30.0);
            let max = args.max.unwrap_or(5).clamp(1, 20) as usize;
            let mut hits: Vec<(&'static str, f64, f64, &'static str)> = STARS
                .iter()
                .filter_map(|(name, ra, dec, mag, desig)| {
                    let d = angular_sep(args.ra_deg, args.dec_deg, *ra, *dec);
                    if d <= tol {
                        Some((*name, d, *mag, *desig))
                    } else {
                        None
                    }
                })
                .collect();
            hits.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            if hits.is_empty() {
                return Ok(text_result(format!(
                    "No catalog star within {tol:.2}° of RA {:.3}°, Dec {:.3}°.",
                    args.ra_deg, args.dec_deg
                )));
            }
            let mut out = format!(
                "Stars near RA {:.3}°, Dec {:.3}° (tolerance {:.2}°):\n",
                args.ra_deg, args.dec_deg, tol
            );
            for (name, d, mag, desig) in hits.iter().take(max) {
                out.push_str(&format!(
                    "  {:<12}  {:>5.2}° away  V {:>5.2}  {}\n",
                    name, d, mag, desig
                ));
            }
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Near Sirius (J2000)",
                args: r#"{"ra_deg": 101.3, "dec_deg": -16.7}"#,
                note: Some("Default 5° tolerance; nearest catalog match returned first."),
            },
            SkillExample {
                title: "Tight tolerance, more candidates",
                args: r#"{"ra_deg": 78.6, "dec_deg": -8.2, "tolerance_deg": 1.0, "max": 3}"#,
                note: Some("Tolerance is capped at 30°; `max` is capped at 20."),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Identify the catalog star nearest a known RA / Dec direction.",
            "Sanity-check a plate-solve solution by labeling the brightest match.",
            "Find catalog candidates within a tolerance of a guessed pointing.",
        ]
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(AstroSun),
        Box::new(AstroMoon),
        Box::new(AstroStar),
        Box::new(AstroStarList),
        Box::new(AstroVisibleStars),
        Box::new(AstroIdentify),
    ]
}
