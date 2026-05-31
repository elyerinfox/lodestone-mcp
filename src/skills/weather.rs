//! Weather skills — keyless point queries against Open-Meteo's free API,
//! which aggregates the same underlying numerical-weather-prediction models
//! Ventusky visualizes (GFS, GEM, ICON, ECMWF IFS, JMA, MetOffice, MET Norway,
//! etc.) plus the ERA5 reanalysis archive, marine forecasts, and air quality.
//!
//! For visual layers Ventusky overlays — radar composites (EURAD/USRAD/WORAD)
//! and live satellite imagery — those are PNG tile services, not point APIs;
//! pull them via `fetch_page` / `webpage_to_pdf` if you need an image.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, send_json_ctx, Skill, SkillCtx};
use crate::text_result;
use crate::util::url_enc;

async fn fetch(server: &crate::Lodestone, url: &str) -> Result<Value, McpError> {
    send_json_ctx(
        server.http.get(url).header("Accept", "application/json"),
        "open-meteo",
    )
    .await
}

/// Render an Open-Meteo hourly block as a fixed-width table, capped at `max`
/// rows. Reads `hourly.time` as the row labels and every other key as a column.
fn render_hourly(v: &Value, title: &str, max: usize) -> String {
    let h = match v.get("hourly").and_then(|x| x.as_object()) {
        Some(o) => o,
        None => return format!("{title}: no hourly block returned."),
    };
    let times: Vec<&str> = h
        .get("time")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|t| t.as_str()).collect())
        .unwrap_or_default();
    let cols: Vec<(&String, &Vec<Value>)> = h
        .iter()
        .filter_map(|(k, v)| {
            if k == "time" {
                None
            } else {
                v.as_array().map(|a| (k, a))
            }
        })
        .collect();
    let n = times.len().min(max);
    let units = v
        .get("hourly_units")
        .and_then(|u| u.as_object())
        .cloned()
        .unwrap_or_default();
    let mut header = String::from("  time             ");
    for (k, _) in &cols {
        let u = units.get(*k).and_then(|x| x.as_str()).unwrap_or("");
        header.push_str(&format!(" | {k} [{u}]"));
    }
    let mut out = format!(
        "{title} (first {n} hours of {} returned):\n{header}\n",
        times.len()
    );
    for (i, t) in times.iter().take(n).enumerate() {
        let mut row = format!("  {:<16}", t);
        for (_, arr) in &cols {
            let val = arr
                .get(i)
                .map(|v| {
                    if let Some(f) = v.as_f64() {
                        format!("{f:.2}")
                    } else if let Some(s) = v.as_str() {
                        s.to_string()
                    } else {
                        v.to_string()
                    }
                })
                .unwrap_or_default();
            row.push_str(&format!(" | {val:>10}"));
        }
        out.push_str(&row);
        out.push('\n');
    }
    if times.len() > n {
        out.push_str(&format!("  … {} more hours truncated\n", times.len() - n));
    }
    out
}

fn render_daily(v: &Value, title: &str, max: usize) -> String {
    let d = match v.get("daily").and_then(|x| x.as_object()) {
        Some(o) => o,
        None => return format!("{title}: no daily block returned."),
    };
    let times: Vec<&str> = d
        .get("time")
        .and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|t| t.as_str()).collect())
        .unwrap_or_default();
    let cols: Vec<(&String, &Vec<Value>)> = d
        .iter()
        .filter_map(|(k, v)| {
            if k == "time" {
                None
            } else {
                v.as_array().map(|a| (k, a))
            }
        })
        .collect();
    let units = v
        .get("daily_units")
        .and_then(|u| u.as_object())
        .cloned()
        .unwrap_or_default();
    let mut header = String::from("  date        ");
    for (k, _) in &cols {
        let u = units.get(*k).and_then(|x| x.as_str()).unwrap_or("");
        header.push_str(&format!(" | {k} [{u}]"));
    }
    let n = times.len().min(max);
    let mut out = format!(
        "{title} (first {n} days of {} returned):\n{header}\n",
        times.len()
    );
    for (i, t) in times.iter().take(n).enumerate() {
        let mut row = format!("  {:<10}", t);
        for (_, arr) in &cols {
            let val = arr
                .get(i)
                .map(|v| {
                    if let Some(f) = v.as_f64() {
                        format!("{f:.2}")
                    } else if let Some(s) = v.as_str() {
                        s.to_string()
                    } else {
                        v.to_string()
                    }
                })
                .unwrap_or_default();
            row.push_str(&format!(" | {val:>10}"));
        }
        out.push_str(&row);
        out.push('\n');
    }
    out
}

// ----- weather_forecast -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ForecastArgs {
    lat: f64,
    lon: f64,
    /// Comma-separated hourly variables. Common: `temperature_2m,relative_humidity_2m,
    /// dew_point_2m,apparent_temperature,precipitation,rain,snowfall,pressure_msl,
    /// cloud_cover,wind_speed_10m,wind_direction_10m,wind_gusts_10m,visibility,
    /// surface_pressure,shortwave_radiation`. Default a sensible surface set.
    #[serde(default)]
    hourly: Option<String>,
    /// Comma-separated daily variables, e.g. `temperature_2m_max,temperature_2m_min,
    /// precipitation_sum,wind_speed_10m_max,sunrise,sunset`. Default none (skip).
    #[serde(default)]
    daily: Option<String>,
    /// Numerical model: `best_match` (default), `gfs_seamless`, `ecmwf_ifs04` /
    /// `ecmwf_ifs025`, `icon_seamless`, `gem_seamless`, `jma_seamless`,
    /// `metno_seamless`, `ukmo_seamless`, `arpege_seamless`, …
    #[serde(default)]
    model: Option<String>,
    /// Forecast horizon in days (default 7, capped at 16).
    #[serde(default)]
    forecast_days: Option<u32>,
    /// Max hourly rows to print (default 48, capped at 384).
    #[serde(default)]
    hours: Option<u32>,
    /// Timezone (default `UTC`). Accepts IANA names or `auto` for local.
    #[serde(default)]
    timezone: Option<String>,
}

const DEFAULT_HOURLY: &str = "temperature_2m,apparent_temperature,relative_humidity_2m,precipitation,wind_speed_10m,wind_direction_10m,wind_gusts_10m,pressure_msl,cloud_cover";

pub struct WeatherForecast;
impl Skill for WeatherForecast {
    fn name(&self) -> &'static str {
        "weather_forecast"
    }
    fn description(&self) -> &'static str {
        "Point weather forecast via Open-Meteo (keyless). Selectable NWP model: best_match \
        (default), gfs_seamless, ecmwf_ifs04/025, icon_seamless, gem_seamless, jma_seamless, \
        metno_seamless, ukmo_seamless, arpege_seamless — the same models Ventusky aggregates. \
        Pass `hourly` and/or `daily` as comma-separated variable lists; defaults to a sensible \
        surface set."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ForecastArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ForecastArgs>()?;
            let hourly = args.hourly.unwrap_or_else(|| DEFAULT_HOURLY.to_string());
            let days = args.forecast_days.unwrap_or(7).clamp(1, 16);
            let hours_cap = args.hours.unwrap_or(48).clamp(1, 384) as usize;
            let tz = args.timezone.unwrap_or_else(|| "UTC".into());
            let model = args.model.unwrap_or_else(|| "best_match".into());
            let mut url = format!(
                "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&hourly={}&forecast_days={days}&timezone={}&models={}",
                args.lat,
                args.lon,
                url_enc(&hourly),
                url_enc(&tz),
                url_enc(model.trim())
            );
            if let Some(d) = args
                .daily
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                url.push_str(&format!("&daily={}", url_enc(d)));
            }
            let v = fetch(server, &url).await?;
            let mut out = format!(
                "Open-Meteo forecast @ ({:.4}, {:.4}) · model={} · tz={}\n",
                args.lat, args.lon, model, tz
            );
            out.push_str(&render_hourly(&v, "Hourly", hours_cap));
            if v.get("daily").is_some() {
                out.push('\n');
                out.push_str(&render_daily(&v, "Daily", days as usize));
            }
            Ok(text_result(out))
        })
    }
}

// ----- weather_archive (ERA5 reanalysis) -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ArchiveArgs {
    lat: f64,
    lon: f64,
    /// `YYYY-MM-DD` start.
    start_date: String,
    /// `YYYY-MM-DD` end.
    end_date: String,
    /// Comma-separated hourly variables; default same set as the forecast.
    #[serde(default)]
    hourly: Option<String>,
    /// Max hourly rows to print (default 72, capped at 1000).
    #[serde(default)]
    hours: Option<u32>,
    #[serde(default)]
    timezone: Option<String>,
}

pub struct WeatherArchive;
impl Skill for WeatherArchive {
    fn name(&self) -> &'static str {
        "weather_archive"
    }
    fn description(&self) -> &'static str {
        "Historical hourly observations / ERA5 reanalysis for a point (Open-Meteo archive, \
        keyless). Pass `start_date` and `end_date` (YYYY-MM-DD) and a set of `hourly` variables. \
        ERA5 covers 1940–present at ~9 km resolution."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ArchiveArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ArchiveArgs>()?;
            let hourly = args.hourly.unwrap_or_else(|| DEFAULT_HOURLY.to_string());
            let hours_cap = args.hours.unwrap_or(72).clamp(1, 1000) as usize;
            let tz = args.timezone.unwrap_or_else(|| "UTC".into());
            let url = format!(
                "https://archive-api.open-meteo.com/v1/archive?latitude={}&longitude={}&start_date={}&end_date={}&hourly={}&timezone={}",
                args.lat,
                args.lon,
                url_enc(args.start_date.trim()),
                url_enc(args.end_date.trim()),
                url_enc(&hourly),
                url_enc(&tz)
            );
            let v = fetch(server, &url).await?;
            let mut out = format!(
                "ERA5 archive @ ({:.4}, {:.4}) · {} → {} · tz={}\n",
                args.lat, args.lon, args.start_date, args.end_date, tz
            );
            out.push_str(&render_hourly(&v, "Hourly", hours_cap));
            Ok(text_result(out))
        })
    }
}

// ----- weather_marine -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct MarineArgs {
    lat: f64,
    lon: f64,
    /// Default: `wave_height,wave_direction,wave_period,wind_wave_height,swell_wave_height`.
    #[serde(default)]
    hourly: Option<String>,
    #[serde(default)]
    hours: Option<u32>,
    #[serde(default)]
    timezone: Option<String>,
}

pub struct WeatherMarine;
impl Skill for WeatherMarine {
    fn name(&self) -> &'static str {
        "weather_marine"
    }
    fn description(&self) -> &'static str {
        "Marine forecast (waves, swell, sea surface) for a coastal/oceanic lat/lon via \
        Open-Meteo (keyless). Same source class Ventusky uses for its waves layer."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<MarineArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<MarineArgs>()?;
            let hourly = args.hourly.unwrap_or_else(|| {
                "wave_height,wave_direction,wave_period,wind_wave_height,swell_wave_height,swell_wave_period".into()
            });
            let hours_cap = args.hours.unwrap_or(48).clamp(1, 384) as usize;
            let tz = args.timezone.unwrap_or_else(|| "UTC".into());
            let url = format!(
                "https://marine-api.open-meteo.com/v1/marine?latitude={}&longitude={}&hourly={}&timezone={}",
                args.lat,
                args.lon,
                url_enc(&hourly),
                url_enc(&tz)
            );
            let v = fetch(server, &url).await?;
            Ok(text_result(format!(
                "Marine forecast @ ({:.4}, {:.4}) · tz={tz}\n{}",
                args.lat,
                args.lon,
                render_hourly(&v, "Hourly", hours_cap)
            )))
        })
    }
}

// ----- weather_air_quality -----

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AqArgs {
    lat: f64,
    lon: f64,
    /// Default: `pm10,pm2_5,carbon_monoxide,ozone,nitrogen_dioxide,sulphur_dioxide,european_aqi,
    /// us_aqi`. Pollen also available: `alder_pollen,birch_pollen,grass_pollen,olive_pollen,mugwort_pollen,ragweed_pollen`.
    #[serde(default)]
    hourly: Option<String>,
    #[serde(default)]
    hours: Option<u32>,
    #[serde(default)]
    timezone: Option<String>,
}

pub struct WeatherAirQuality;
impl Skill for WeatherAirQuality {
    fn name(&self) -> &'static str {
        "weather_air_quality"
    }
    fn description(&self) -> &'static str {
        "Air-quality (PM2.5/PM10/O3/NO2/SO2/CO + US/European AQI) and pollen forecast for a \
        lat/lon via Open-Meteo (keyless, CAMS-backed). Same source class Ventusky uses for its \
        air-quality / pollen overlays."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<AqArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<AqArgs>()?;
            let hourly = args.hourly.unwrap_or_else(|| {
                "pm10,pm2_5,carbon_monoxide,ozone,nitrogen_dioxide,sulphur_dioxide,european_aqi,us_aqi".into()
            });
            let hours_cap = args.hours.unwrap_or(48).clamp(1, 384) as usize;
            let tz = args.timezone.unwrap_or_else(|| "UTC".into());
            let url = format!(
                "https://air-quality-api.open-meteo.com/v1/air-quality?latitude={}&longitude={}&hourly={}&timezone={}",
                args.lat,
                args.lon,
                url_enc(&hourly),
                url_enc(&tz)
            );
            let v = fetch(server, &url).await?;
            Ok(text_result(format!(
                "Air quality @ ({:.4}, {:.4}) · tz={tz}\n{}",
                args.lat,
                args.lon,
                render_hourly(&v, "Hourly", hours_cap)
            )))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(WeatherForecast),
        Box::new(WeatherArchive),
        Box::new(WeatherMarine),
        Box::new(WeatherAirQuality),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny inline Open-Meteo response shape; render_hourly should produce a
    /// fixed-width table with units pulled from `hourly_units` and rows for
    /// each timestamp.
    fn fixture_hourly() -> Value {
        serde_json::json!({
            "hourly_units": {"temperature_2m": "°C", "wind_speed_10m": "km/h"},
            "hourly": {
                "time": ["2026-05-30T00:00", "2026-05-30T01:00", "2026-05-30T02:00"],
                "temperature_2m": [12.3, 12.1, 11.9],
                "wind_speed_10m": [8.0, 10.5, 11.2]
            }
        })
    }

    #[test]
    fn render_hourly_pulls_units_and_caps_rows() {
        let v = fixture_hourly();
        let out = render_hourly(&v, "Forecast", 2);
        assert!(out.contains("Forecast (first 2 hours of 3 returned)"));
        assert!(out.contains("temperature_2m [°C]"));
        assert!(out.contains("wind_speed_10m [km/h]"));
        // Row count: 2 data rows + the truncation footer.
        assert!(out.contains("2026-05-30T00:00"));
        assert!(out.contains("2026-05-30T01:00"));
        assert!(!out.contains("2026-05-30T02:00"));
        assert!(out.contains("1 more hours truncated"));
        // Numeric values formatted to 2 decimals.
        assert!(out.contains("12.30"));
        assert!(out.contains("10.50"));
    }

    #[test]
    fn render_hourly_handles_missing_block() {
        let v = serde_json::json!({});
        let out = render_hourly(&v, "Forecast", 10);
        assert!(out.contains("no hourly block returned"));
    }

    #[test]
    fn render_daily_basic() {
        let v = serde_json::json!({
            "daily_units": {"temperature_2m_max": "°C", "sunrise": "iso8601"},
            "daily": {
                "time": ["2026-05-30", "2026-05-31"],
                "temperature_2m_max": [22.5, 24.1],
                "sunrise": ["2026-05-30T05:30", "2026-05-31T05:29"]
            }
        });
        let out = render_daily(&v, "Daily", 10);
        assert!(out.contains("temperature_2m_max [°C]"));
        assert!(out.contains("sunrise [iso8601]"));
        assert!(out.contains("2026-05-30"));
        assert!(out.contains("22.50"));
        // String values pass through.
        assert!(out.contains("2026-05-30T05:30"));
    }

    fn http() -> reqwest::Client {
        crate::skills::live_http()
    }

    /// Live forecast call — Redmond, WA, 1-hour horizon, single variable.
    #[tokio::test]
    #[ignore]
    async fn open_meteo_forecast_live() {
        let url = "https://api.open-meteo.com/v1/forecast?latitude=47.67&longitude=-122.12&hourly=temperature_2m&forecast_days=1&timezone=UTC&models=best_match";
        let r = http()
            .get(url)
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: Value = r.json().await.unwrap();
        assert!(v.get("hourly").is_some(), "no hourly block");
        assert!(v["hourly"]["time"].as_array().unwrap().len() >= 24);
        assert!(v["hourly"]["temperature_2m"].as_array().unwrap().len() >= 24);
    }

    /// Live ERA5 archive — yesterday only, one variable.
    #[tokio::test]
    #[ignore]
    async fn open_meteo_archive_live() {
        let url = "https://archive-api.open-meteo.com/v1/archive?latitude=47.67&longitude=-122.12&start_date=2024-01-01&end_date=2024-01-01&hourly=temperature_2m&timezone=UTC";
        let r = http()
            .get(url)
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["hourly"]["time"].as_array().unwrap().len(), 24);
    }

    #[tokio::test]
    #[ignore]
    async fn open_meteo_marine_live() {
        // Open ocean coordinates so we definitely get wave data.
        let url = "https://marine-api.open-meteo.com/v1/marine?latitude=36.7&longitude=-122.3&hourly=wave_height";
        let r = http()
            .get(url)
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: Value = r.json().await.unwrap();
        assert!(v["hourly"]["wave_height"].is_array());
    }

    #[tokio::test]
    #[ignore]
    async fn open_meteo_air_quality_live() {
        let url = "https://air-quality-api.open-meteo.com/v1/air-quality?latitude=47.67&longitude=-122.12&hourly=pm2_5";
        let r = http()
            .get(url)
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: Value = r.json().await.unwrap();
        assert!(v["hourly"]["pm2_5"].is_array());
    }
}
