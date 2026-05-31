//! U.S. Energy Information Administration (EIA) skills — query the v2 Open
//! Data API at `api.eia.gov/v2/`. Time-series datasets covering electricity,
//! natural gas, petroleum, coal, renewables, international, and more.
//!
//! Requires a free EIA API key in `[eia].key` (or `LODESTONE_EIA_KEY`).
//! Get one at <https://www.eia.gov/opendata/register.php>.

use std::sync::Arc;

use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;
use serde_json::Value;

use crate::skills::{schema_for, send_json_ctx, Skill, SkillCtx};
use crate::{invalid, text_result};

fn url_enc(s: &str) -> String {
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SeriesArgs {
    /// Dataset path under `/v2/`, e.g. `electricity/retail-sales/data`,
    /// `petroleum/pri/gnd/data`, `natural-gas/pri/sum/data`, `total-energy/data`.
    /// Browse at https://www.eia.gov/opendata/browser/.
    path: String,
    /// Frequency: `annual` / `monthly` / `quarterly` / `daily` / `hourly` (varies
    /// by dataset; defaults to the dataset's coarsest).
    #[serde(default)]
    frequency: Option<String>,
    /// Comma-separated facet filters like `stateid=WA,sectorid=RES`.
    #[serde(default)]
    facets: Option<String>,
    /// Comma-separated data columns to request (e.g. `value,price`). Default `value`.
    #[serde(default)]
    data: Option<String>,
    /// `YYYY-MM-DD` (or `YYYY-MM` / `YYYY` depending on freq) — start.
    #[serde(default)]
    start: Option<String>,
    /// End date.
    #[serde(default)]
    end: Option<String>,
    /// Max rows to return (default 100, capped at 5000).
    #[serde(default)]
    length: Option<u32>,
}

pub struct EiaSeries;
impl Skill for EiaSeries {
    fn name(&self) -> &'static str {
        "eia_series"
    }
    fn description(&self) -> &'static str {
        "Query the EIA v2 Open Data API for a time-series dataset. Pass the dataset `path` \
        (e.g. `electricity/retail-sales/data`), optional `frequency`, `facets` \
        (`key=val,key=val`), `data` columns, `start`/`end`, and `length`. Requires `[eia].key`. \
        Browse datasets at https://www.eia.gov/opendata/browser/."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SeriesArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SeriesArgs>()?;
            let key = server.eia_key.as_ref();
            if key.is_empty() {
                return Err(invalid(
                    "no EIA API key configured — set `[eia].key` or LODESTONE_EIA_KEY",
                ));
            }
            let path = args.path.trim().trim_matches('/');
            let length = args.length.unwrap_or(100).clamp(1, 5000);
            let data_cols = args.data.unwrap_or_else(|| "value".into());
            let mut url = format!("https://api.eia.gov/v2/{path}/?api_key={key}&length={length}");
            if let Some(f) = &args.frequency {
                url.push_str(&format!("&frequency={}", url_enc(f.trim())));
            }
            for col in data_cols.split(',') {
                let c = col.trim();
                if !c.is_empty() {
                    url.push_str(&format!("&data[]={}", url_enc(c)));
                }
            }
            if let Some(facets) = &args.facets {
                for f in facets.split(',') {
                    if let Some((k, v)) = f.split_once('=') {
                        url.push_str(&format!(
                            "&facets[{}][]={}",
                            url_enc(k.trim()),
                            url_enc(v.trim())
                        ));
                    }
                }
            }
            if let Some(s) = &args.start {
                url.push_str(&format!("&start={}", url_enc(s.trim())));
            }
            if let Some(e) = &args.end {
                url.push_str(&format!("&end={}", url_enc(e.trim())));
            }
            let v: Value = send_json_ctx(
                server.http.get(&url).header("Accept", "application/json"),
                "eia",
            )
            .await?;
            let response = v.get("response").unwrap_or(&v);
            let total = response.get("total").and_then(|x| x.as_i64()).unwrap_or(-1);
            let empty = Vec::new();
            let data = response
                .get("data")
                .and_then(|x| x.as_array())
                .unwrap_or(&empty);
            let mut out = format!("EIA {path}: {} rows (total {total})\n", data.len());
            for row in data.iter().take(50) {
                if let Some(obj) = row.as_object() {
                    let parts: Vec<String> = obj.iter().map(|(k, v)| format!("{k}={v}")).collect();
                    out.push_str(&format!("  {}\n", parts.join("  ")));
                }
            }
            if data.len() > 50 {
                out.push_str(&format!("  … {} more rows truncated\n", data.len() - 50));
            }
            Ok(text_result(out))
        })
    }
}

pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![Box::new(EiaSeries)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_enc_round_trips_safe_and_percent_encodes_special() {
        assert_eq!(url_enc("annual"), "annual");
        assert_eq!(
            url_enc("electricity/retail-sales/data"),
            "electricity%2Fretail-sales%2Fdata"
        );
        assert_eq!(url_enc("stateid=WA"), "stateid%3DWA");
    }

    fn http() -> reqwest::Client {
        crate::skills::live_http()
    }

    /// EIA needs a real API key; skip the live test cleanly when one isn't
    /// configured. CI nightlies can provide LODESTONE_EIA_KEY.
    #[tokio::test]
    #[ignore]
    async fn eia_v2_live() {
        let key = match std::env::var("LODESTONE_EIA_KEY").or_else(|_| std::env::var("EIA_API_KEY"))
        {
            Ok(k) if !k.trim().is_empty() => k,
            _ => {
                eprintln!("skipping eia live: no LODESTONE_EIA_KEY/EIA_API_KEY");
                return;
            }
        };
        let url = format!("https://api.eia.gov/v2/electricity/retail-sales/data/?api_key={key}&frequency=annual&data[]=price&length=3");
        let r = http()
            .get(&url)
            .send()
            .await
            .expect("network")
            .error_for_status()
            .unwrap();
        let v: Value = r.json().await.unwrap();
        assert!(v.get("response").is_some(), "missing response envelope");
        let data = v["response"]["data"].as_array().expect("no data array");
        assert!(!data.is_empty(), "data array empty");
    }
}
