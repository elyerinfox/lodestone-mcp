//! NOAA / NWS weather skills — keyless public APIs at `api.weather.gov`.
//! Active weather alerts and point-forecast (U.S. coverage). For NESDIS
//! satellite imagery and global products, the data is download-oriented and
//! best fetched via the existing `fetch_page` / `read_pdf` / `store_*` tools
//! against the NESDIS catalog (<https://www.nesdis.noaa.gov/>).

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, Skill, SkillCtx};
use crate::{internal, invalid, text_result};

async fn fetch(server: &crate::Lodestone, url: &str) -> Result<Value, McpError> {
    let r = server
        .http
        .get(url)
        .header("Accept", "application/geo+json")
        .send()
        .await
        .and_then(|x| x.error_for_status())
        .map_err(|e| internal(anyhow::anyhow!("nws {url}: {e}")))?;
    r.json()
        .await
        .map_err(|e| internal(anyhow::anyhow!("nws parse: {e}")))
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AlertsArgs {
    /// Two-letter state code (e.g. "WA") or `null` for nationwide. Optional.
    #[serde(default)]
    area: Option<String>,
    /// `actual` (default) or `exercise`/`system`/`test`/`draft`.
    #[serde(default)]
    status: Option<String>,
    /// Max alerts to summarize (default 25, capped at 200).
    #[serde(default)]
    max: Option<u32>,
}

pub struct NoaaAlerts;
impl Skill for NoaaAlerts {
    fn name(&self) -> &'static str {
        "noaa_alerts"
    }
    fn description(&self) -> &'static str {
        "Active U.S. weather alerts from the NWS (api.weather.gov, keyless). Filter by `area` \
        (two-letter state code) or omit for nationwide. Returns event, severity, area, and \
        the headline."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<AlertsArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<AlertsArgs>()?;
            let max = args.max.unwrap_or(25).clamp(1, 200) as usize;
            let status = args
                .status
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("actual");
            let mut url = format!("https://api.weather.gov/alerts/active?status={status}");
            if let Some(a) = args
                .area
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                url.push_str(&format!("&area={}", a.to_ascii_uppercase()));
            }
            let v = fetch(server, &url).await?;
            let empty = Vec::new();
            let features = v
                .get("features")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            if features.is_empty() {
                return Ok(text_result("No active alerts.".to_string()));
            }
            let mut out = format!("{} active alert(s):\n", features.len());
            for f in features.iter().take(max) {
                let p = f.get("properties").cloned().unwrap_or(Value::Null);
                let event = p.get("event").and_then(|x| x.as_str()).unwrap_or("?");
                let sev = p.get("severity").and_then(|x| x.as_str()).unwrap_or("?");
                let area = p.get("areaDesc").and_then(|x| x.as_str()).unwrap_or("?");
                let head = p.get("headline").and_then(|x| x.as_str()).unwrap_or("");
                out.push_str(&format!(
                    "  [{sev}] {event}\n    area: {area}\n    {head}\n"
                ));
            }
            if features.len() > max {
                out.push_str(&format!("  … {} more truncated\n", features.len() - max));
            }
            Ok(text_result(out))
        })
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ForecastArgs {
    /// Latitude (U.S. coverage).
    lat: f64,
    /// Longitude.
    lon: f64,
    /// `hourly` (default) or `daily`.
    #[serde(default)]
    period: Option<String>,
    /// Max periods to summarize (default 12, capped at 168).
    #[serde(default)]
    max: Option<u32>,
}

pub struct NoaaForecast;
impl Skill for NoaaForecast {
    fn name(&self) -> &'static str {
        "noaa_forecast"
    }
    fn description(&self) -> &'static str {
        "NWS point forecast for a U.S. lat/lon (keyless). `period` = `hourly` (default) or \
        `daily`. Returns temperature, wind, and a short forecast for each period. Two-step \
        under the hood: /points/{lat},{lon} → gridpoint forecast URL."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ForecastArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ForecastArgs>()?;
            let max = args.max.unwrap_or(12).clamp(1, 168) as usize;
            let period = args
                .period
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("hourly")
                .to_ascii_lowercase();
            let points_url = format!(
                "https://api.weather.gov/points/{:.4},{:.4}",
                args.lat, args.lon
            );
            let points = fetch(server, &points_url).await?;
            let prop = points.get("properties").cloned().unwrap_or(Value::Null);
            let url_key = if period == "daily" {
                "forecast"
            } else {
                "forecastHourly"
            };
            let forecast_url = prop
                .get(url_key)
                .and_then(|x| x.as_str())
                .ok_or_else(|| invalid("point is outside NWS coverage (U.S. only)"))?;
            let fc = fetch(server, forecast_url).await?;
            let empty = Vec::new();
            let periods = fc
                .get("properties")
                .and_then(|p| p.get("periods"))
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            let mut out = format!(
                "NWS {period} forecast for ({:.4}, {:.4}) — {} period(s) shown:\n",
                args.lat,
                args.lon,
                periods.len().min(max)
            );
            for p in periods.iter().take(max) {
                let name = p.get("name").and_then(|x| x.as_str()).unwrap_or("");
                let start = p.get("startTime").and_then(|x| x.as_str()).unwrap_or("");
                let temp = p.get("temperature").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let tu = p
                    .get("temperatureUnit")
                    .and_then(|x| x.as_str())
                    .unwrap_or("F");
                let wind = p.get("windSpeed").and_then(|x| x.as_str()).unwrap_or("");
                let wd = p
                    .get("windDirection")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                let short = p
                    .get("shortForecast")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                out.push_str(&format!(
                    "  {start}  ({name})  {temp:.0}°{tu}  wind {wind} {wd}  · {short}\n"
                ));
            }
            Ok(text_result(out))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(NoaaAlerts), Box::new(NoaaForecast)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .user_agent(crate::LODESTONE_UA)
            .build()
            .unwrap()
    }

    /// The /alerts endpoint returns a GeoJSON FeatureCollection even when
    /// there's nothing active — verify the envelope.
    #[tokio::test]
    #[ignore]
    async fn nws_alerts_live() {
        let r = http()
            .get("https://api.weather.gov/alerts/active?status=actual&area=WA")
            .header("Accept", "application/geo+json")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["type"].as_str(), Some("FeatureCollection"));
        assert!(v["features"].is_array());
    }

    /// The two-step /points → forecast handoff is the brittle bit:
    /// the /points response embeds the gridpoint forecast URL we follow.
    #[tokio::test]
    #[ignore]
    async fn nws_points_then_forecast_live() {
        let c = http();
        let p = c
            .get("https://api.weather.gov/points/47.6700,-122.1200")
            .header("Accept", "application/geo+json")
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let pv: Value = p.json().await.unwrap();
        let fc_url = pv["properties"]["forecastHourly"]
            .as_str()
            .expect("forecastHourly missing — /points contract change");
        let fc = c
            .get(fc_url)
            .header("Accept", "application/geo+json")
            .send()
            .await
            .expect("forecast network")
            .error_for_status()
            .unwrap();
        let fv: Value = fc.json().await.unwrap();
        // Each period carries the keys our renderer relies on.
        let p0 = &fv["properties"]["periods"][0];
        for k in [
            "startTime",
            "temperature",
            "temperatureUnit",
            "windSpeed",
            "shortForecast",
        ] {
            assert!(
                p0.get(k).is_some(),
                "missing key {k} in /forecast/hourly period"
            );
        }
    }
}
