# U.S. Energy Information Administration — `eia_*`

|  |  |
| --- | --- |
| **Module** | [`src/skills/eia.rs`](../../src/skills/eia.rs) |
| **Tools** | `eia_browse`, `eia_series` |
| **Network** | `api.eia.gov/v2` — requires a **free** API key |
| **Default** | **on** when `[eia].key` is set; tools are inert otherwise |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); key via `LODESTONE_EIA_KEY` / `EIA_API_KEY`. Register at <https://www.eia.gov/opendata/register.php>. Defaults in [`src/config.rs`](../../src/config.rs). |

## What it does

Query the EIA **v2** Open Data API — time-series datasets covering
**electricity, natural gas, petroleum, coal, renewables, international**, and
more. The API requires a free key (registration takes ~30 seconds).

## Tools

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `eia_browse` | `path?` | Browse the dataset tree starting at `path` (default root). Discover series ids by walking the hierarchy. |
| `eia_series` | `path`, `frequency?`, `data?`, `facets?`, `start?`, `end?` | Pull a specific time series. |

## Example uses

- **Browse** — `eia_browse {}` → top-level routes; drill down with
  `eia_browse { path: "electricity" }`.
- **Series** —
  `eia_series { path: "electricity/retail-sales/data", frequency: "annual", facets: { stateid: "WA" } }`.

## Notes

- **Key required**, but free and unmetered for typical use. Without
  `[eia].key` the `eia_*` tools are inert (the [`features`](meta.md) tool
  shows them as such).
- **Schemas vary by series.** `eia_browse` is the discovery path; once you
  have a `path` and its facet schema, `eia_series` pulls rows.

## See also

- [tools.md](../tools.md)
- [skills/grid.md](grid.md) — physical power infrastructure locations.
- [skills/weather.md](weather.md) — load is weather-driven; correlate
  forecasts with consumption.
