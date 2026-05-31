# HTML render & diagnostics — `html_render`

|  |  |
| --- | --- |
| **Module** | [`src/skills/html.rs`](../../src/skills/html.rs) |
| **Tools** | `html_render` |
| **Network** | the page's own fetches (rendered locally; nothing leaves the host except whatever the page itself requests) |
| **Default** | **on** — gated by `[html]` |
| **Config** | gate via `[tools]` ([`config/01-tools.toml`](../../config/01-tools.toml)); `[html].enabled` via `LODESTONE_HTML_ENABLED`. Shares the headless browser with `[google]` (`chrome_path`, `no_sandbox`, `args`). Defaults in [`src/config.rs`](../../src/config.rs). |

## What it does

Executes an HTML snippet OR navigates to a URL in the **shared headless Chrome**
the rest of the project uses (the same browser as `render_page` and the search
provider rendering), waits a configurable number of milliseconds for JavaScript
to run, then returns a structured diagnostics report:

- **Console** — every `console.log` / `info` / `warn` / `error` / `debug` /
  `trace` / `dir` / `table` / `count` / `time` / `group` / `clear` / `assert` /
  `profile` call. Level, concatenated args, source URL + 1-based line number
  from the CDP stack-trace top frame.
- **JS exceptions** — every `Runtime.exceptionThrown` event, with text,
  source / line / column, and a flattened multi-frame stack trace.
- **Network failures** — every `Network.loadingFailed` event (DNS, connection
  refused, CORS block, ad-blocker interception, mixed-content block, …).
  Distinguished from HTTP errors because no response was ever received.
- **HTTP errors** — every response with status ≥ 400, with URL, status, and
  resource type.
- **Summary** — final page title, final URL after redirects, total elapsed time.

Use it **after `chart_interactive`** (or any HTML-emitting tool) to verify the
output actually runs cleanly before shipping it to the model's caller. A
clean run is one with `0 console errors · 0 JS exceptions · 0 network failures
· 0 HTTP errors`.

## Tool

| Tool | Arguments | Purpose |
| --- | --- | --- |
| `html_render` | `html?`, `url?`, `wait_ms?` | Render HTML / URL and capture diagnostics. Exactly one of `html` or `url` is required; `wait_ms` defaults to `1500`, capped at `30000`. |

## Example uses

- **Verify a generated chart** —
  `chart_interactive { library: "chartjs", config: { … } }` → take its HTML →
  `html_render { html: "<…>" }`. If diagnostics show `0` of each, the
  output is safe to send on.
- **Inspect a URL's runtime behavior** —
  `html_render { url: "https://example.com/", wait_ms: 3000 }`.
- **Reproduce a console error** — render a minimal repro snippet and read the
  console event with source/line.

## Implementation notes

The implementation subscribes to CDP event streams **before** navigation
(`Runtime.consoleAPICalled`, `Runtime.exceptionThrown`,
`Network.loadingFailed`, `Network.responseReceived`), collects them into
`Arc<Mutex<Vec<…>>>` buffers via spawned tasks during the wait, then drains
and closes the page. Subscribing first matters: a `console.error` that fires
inside an inline `<script>` would otherwise race against the subscription and
get lost.

## Notes

- **Requires Chrome / Chromium** on `PATH` (or `[google].chrome_path`). The
  binary is the same one search-provider rendering and `render_page` use.
- **`wait_ms` capped at 30 s.** Long-running pages should be split into smaller
  reproductions.
- **JS exceptions are uncaught only.** Caught exceptions don't surface here;
  if you need them, `console.error(e)` inside the catch.

## See also

- [tools.md](../tools.md)
- [skills/chart.md](chart.md) — `chart_interactive` is the most common reason
  to reach for `html_render`.
- [skills/retrieve.md](retrieve.md) — `render_page` for "scrape this page as
  text" rather than "did this page run cleanly".
