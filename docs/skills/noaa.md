# NOAA / NWS weather — `noaa_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/noaa.rs`](../../src/skills/noaa.rs) |
| **Tools** | `noaa_alerts`, `noaa_forecast` |
| **Network** | `api.weather.gov` — **keyless** |
| **Default** | **on** |
| **Config** | none |

## What it does

NOAA / NWS active weather alerts and point forecasts for **US coverage** via
the keyless `api.weather.gov` API. For NESDIS satellite imagery and global
products, the data is download-oriented; fetch via `fetch_page` / `read_pdf` /
`store_*` against the NESDIS catalog.

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `noaa_alerts` | `area?`, `status?`, `max?` | Active alerts. `area`: two-letter state (e.g. `"WA"`) or omit for nationwide. `status`: `actual` (default) / `exercise` / `system` / `test` / `draft`. |
| `noaa_forecast` | `lat`, `lon`, `hourly?` | Point forecast (US). `hourly=true` for hourly resolution; default is twice-daily periods. |

## Example uses

- **Active alerts** — `noaa_alerts { area: "WA" }`.
- **Point forecast** — `noaa_forecast { lat: 47.6, lon: -122.3 }`.

## Notes

- **US only.** Outside US lat/lon, `noaa_forecast` returns an empty / error
  response.
- **Keyless.** NWS asks for a descriptive User-Agent; the project's
  `lodestone-mcp/…` UA satisfies their policy.

## See also

- [tools.md](../tools.md)
- [skills/weather.md](weather.md) — global coverage via Open-Meteo.
