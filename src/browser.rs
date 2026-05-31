//! Headless-browser rendering as its own capability (feature `browser`).
//!
//! A [`PageRenderer`] renders a URL with a real browser — executing JavaScript —
//! and returns the resulting HTML. The Chromium implementation keeps a single
//! persistent browser alive (launching one per request adds ~10s of latency)
//! and is exposed process-wide via [`shared_global`], so the Google provider,
//! the StackOverflow scraper, and on-demand `fetch_page`/search rendering all
//! reuse the same browser.

use std::sync::Arc;
use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use tokio::sync::{RwLock, Semaphore};

/// Renders a page with a real browser and returns its final HTML, or prints it
/// to PDF.
#[async_trait]
pub trait PageRenderer: Send + Sync {
    async fn render(&self, url: &str) -> Result<String>;
    /// Render the page and return it printed to PDF bytes (local, no service).
    async fn render_pdf(&self, url: &str) -> Result<Vec<u8>>;
    /// Render the page and return diagnostics (console events, JS exceptions,
    /// failed network requests, HTTP error responses, title, final URL,
    /// elapsed wall time). Used by `html_render` to verify a UI / chart /
    /// snippet actually runs cleanly. `input` is either a URL (navigated to)
    /// or a raw HTML string (set as the page body). `wait_ms` is how long to
    /// let JavaScript run before snapshotting.
    async fn render_diagnostics(
        &self,
        input: RenderInput<'_>,
        wait_ms: u64,
    ) -> Result<PageDiagnostics>;
}

/// What `render_diagnostics` should load.
#[derive(Debug, Clone, Copy)]
pub enum RenderInput<'a> {
    Url(&'a str),
    Html(&'a str),
}

/// Aggregated diagnostics from a headless browser render. All fields are
/// "what happened during the run" — the snapshot is intentionally bounded
/// in time (caller picks the `wait_ms`).
#[derive(Debug, Default, Clone)]
pub struct PageDiagnostics {
    pub title: String,
    pub final_url: String,
    pub elapsed_ms: u64,
    pub console: Vec<ConsoleMessage>,
    pub exceptions: Vec<JsException>,
    pub network_failures: Vec<NetFailure>,
    pub http_errors: Vec<HttpError>,
}

/// One `console.log/info/warn/error/debug/trace/...` invocation.
#[derive(Debug, Clone)]
pub struct ConsoleMessage {
    /// Console method invoked: `log`, `info`, `warning`, `error`, `debug`, etc.
    pub level: String,
    /// Arguments concatenated to one string. Objects render as JSON; strings
    /// render verbatim.
    pub text: String,
    /// File / URL the call site was in (when CDP attributes one).
    pub source_url: Option<String>,
    /// 1-based line number of the call site, when known.
    pub line: Option<u32>,
}

/// One uncaught JS exception (`Runtime.exceptionThrown`).
#[derive(Debug, Clone)]
pub struct JsException {
    pub text: String,
    pub source_url: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// Flattened stack frames (top frame first), or `None` when CDP didn't
    /// attach a stack trace (e.g. exception thrown without `Error` object).
    pub stack: Option<String>,
}

/// One outright network failure (DNS error, connection refused, mixed-content
/// block, ad-blocker interception, etc.). Distinguished from `HttpError`
/// because no HTTP response was ever received.
#[derive(Debug, Clone)]
pub struct NetFailure {
    pub url: String,
    pub error_text: String,
    pub resource_type: String,
}

/// One HTTP response that returned an error status (4xx / 5xx). The page
/// may still have rendered, but missing assets or API failures show up here.
#[derive(Debug, Clone)]
pub struct HttpError {
    pub url: String,
    pub status: u16,
    pub resource_type: String,
}

/// How the headless browser is launched. Defaults auto-detect Chrome and run a
/// normal sandboxed instance; containers typically set `no_sandbox`.
#[derive(Clone)]
pub struct BrowserOptions {
    /// Path to the Chrome/Chromium executable; empty = auto-detect.
    pub chrome_path: String,
    /// Pass `--no-sandbox` and `--disable-dev-shm-usage`. Required when running
    /// Chrome as root (e.g. inside a Docker container).
    pub no_sandbox: bool,
    /// Additional command-line flags to pass to Chrome.
    pub args: Vec<String>,
    /// Maximum pages (tabs) rendered concurrently on the shared browser. Bounds
    /// memory/CPU; renders beyond this queue for a slot. Clamped to >= 1.
    pub render_concurrency: usize,
}

impl Default for BrowserOptions {
    fn default() -> Self {
        Self {
            chrome_path: String::new(),
            no_sandbox: false,
            args: Vec::new(),
            render_concurrency: 4,
        }
    }
}

/// A [`PageRenderer`] backed by a single persistent headless Chrome/Chromium
/// instance. Renders run as **concurrent pages** on that one browser — bounded by
/// a semaphore (`render_concurrency`) so a burst can't exhaust memory — rather than
/// being serialized one-at-a-time. The browser is shared behind an `RwLock` (read
/// to use it, write only to launch/relaunch).
pub struct ChromiumRenderer {
    options: BrowserOptions,
    browser: RwLock<Option<Arc<BrowserHandle>>>,
    pages: Semaphore,
}

impl ChromiumRenderer {
    pub fn new(options: BrowserOptions) -> Self {
        let permits = options.render_concurrency.max(1);
        Self {
            options,
            browser: RwLock::new(None),
            pages: Semaphore::new(permits),
        }
    }

    /// The live browser, launching it on first use.
    async fn handle(&self) -> Result<Arc<BrowserHandle>> {
        if let Some(h) = self.browser.read().await.as_ref() {
            return Ok(h.clone());
        }
        self.relaunch(None).await
    }

    /// (Re)launch the browser. When `failed` is given, only relaunch if it's still
    /// the current handle — so concurrent renders that all saw the same dead browser
    /// trigger a single relaunch, not one each.
    async fn relaunch(&self, failed: Option<&Arc<BrowserHandle>>) -> Result<Arc<BrowserHandle>> {
        let mut guard = self.browser.write().await;
        if let (Some(cur), Some(failed)) = (guard.as_ref(), failed) {
            if !Arc::ptr_eq(cur, failed) {
                return Ok(cur.clone()); // someone already relaunched
            }
        }
        let handle = Arc::new(launch(&self.options).await?);
        *guard = Some(handle.clone());
        Ok(handle)
    }
}

#[async_trait]
impl PageRenderer for ChromiumRenderer {
    async fn render(&self, url: &str) -> Result<String> {
        let _permit = self.pages.acquire().await;
        let handle = self.handle().await?;
        match render_page(&handle, url).await {
            Ok(html) => Ok(html),
            Err(e) => {
                // The browser may have died; relaunch once and retry.
                tracing::warn!(error = %e, "headless browser failed; relaunching");
                let handle = self.relaunch(Some(&handle)).await?;
                render_page(&handle, url).await
            }
        }
    }

    async fn render_pdf(&self, url: &str) -> Result<Vec<u8>> {
        let _permit = self.pages.acquire().await;
        let handle = self.handle().await?;
        match page_to_pdf(&handle, url).await {
            Ok(bytes) => Ok(bytes),
            Err(e) => {
                tracing::warn!(error = %e, "headless browser failed; relaunching");
                let handle = self.relaunch(Some(&handle)).await?;
                page_to_pdf(&handle, url).await
            }
        }
    }

    async fn render_diagnostics(
        &self,
        input: RenderInput<'_>,
        wait_ms: u64,
    ) -> Result<PageDiagnostics> {
        let _permit = self.pages.acquire().await;
        let handle = self.handle().await?;
        match page_diagnostics(&handle, input, wait_ms).await {
            Ok(d) => Ok(d),
            Err(e) => {
                tracing::warn!(error = %e, "headless browser failed; relaunching");
                let handle = self.relaunch(Some(&handle)).await?;
                page_diagnostics(&handle, input, wait_ms).await
            }
        }
    }
}

static SHARED: OnceLock<ChromiumRenderer> = OnceLock::new();
static OPTIONS: OnceLock<BrowserOptions> = OnceLock::new();

/// Set the browser options used by [`shared_global`]. Call once at startup.
pub fn configure(options: BrowserOptions) {
    let _ = OPTIONS.set(options);
}

/// The process-wide shared renderer, created on first use from the configured
/// [`BrowserOptions`] (any provider can use this to render a page on demand).
pub fn shared_global() -> &'static ChromiumRenderer {
    SHARED.get_or_init(|| ChromiumRenderer::new(OPTIONS.get().cloned().unwrap_or_default()))
}

struct BrowserHandle {
    browser: Browser,
    _driver: tokio::task::JoinHandle<()>,
}

async fn launch(options: &BrowserOptions) -> Result<BrowserHandle> {
    let mut flags: Vec<String> = Vec::new();
    if options.no_sandbox {
        flags.push("--no-sandbox".to_string());
        flags.push("--disable-dev-shm-usage".to_string());
    }
    flags.extend(options.args.iter().cloned());

    let mut builder = BrowserConfig::builder();
    if !options.chrome_path.is_empty() {
        builder = builder.chrome_executable(&options.chrome_path);
    }
    if !flags.is_empty() {
        builder = builder.args(flags);
    }
    let config = builder
        .build()
        .map_err(|e| anyhow!("failed to build browser config: {e}"))?;
    let (browser, mut handler) = Browser::launch(config).await.map_err(|e| {
        anyhow!(
            "headless browser unavailable — could not start Chrome/Chromium. Install Chrome or set \
             [google].chrome_path (LODESTONE_CHROME_PATH); inside containers also set \
             [google].no_sandbox. Underlying error: {e}"
        )
    })?;
    let driver = tokio::spawn(async move { while handler.next().await.is_some() {} });
    Ok(BrowserHandle {
        browser,
        _driver: driver,
    })
}

async fn render_page(handle: &BrowserHandle, url: &str) -> Result<String> {
    let page = handle.browser.new_page(url).await?;
    page.wait_for_navigation().await?;
    let html = page.content().await?;
    let _ = page.close().await;
    Ok(html)
}

async fn page_to_pdf(handle: &BrowserHandle, url: &str) -> Result<Vec<u8>> {
    use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;
    let page = handle.browser.new_page(url).await?;
    page.wait_for_navigation().await?;
    let bytes = page.pdf(PrintToPdfParams::default()).await?;
    let _ = page.close().await;
    Ok(bytes)
}

async fn page_diagnostics(
    handle: &BrowserHandle,
    input: RenderInput<'_>,
    wait_ms: u64,
) -> Result<PageDiagnostics> {
    use std::sync::Mutex;
    use std::time::Instant;

    use chromiumoxide::cdp::browser_protocol::network::{
        EventLoadingFailed, EventResponseReceived,
    };
    use chromiumoxide::cdp::js_protocol::runtime::{EventConsoleApiCalled, EventExceptionThrown};

    let start = Instant::now();
    // Open a blank page first so event listeners are subscribed BEFORE
    // any navigation / content-set runs, otherwise we'd race the load
    // pipeline and miss early console / exception messages.
    let page = handle.browser.new_page("about:blank").await?;

    // Subscribe to the four event streams we care about. Streams are buffered
    // by chromiumoxide internally, so we can drain them later without losing
    // events even though we're not actively polling during the wait.
    let mut console_stream = page.event_listener::<EventConsoleApiCalled>().await?;
    let mut exc_stream = page.event_listener::<EventExceptionThrown>().await?;
    let mut fail_stream = page.event_listener::<EventLoadingFailed>().await?;
    let mut resp_stream = page.event_listener::<EventResponseReceived>().await?;

    let console_buf: Arc<Mutex<Vec<Arc<EventConsoleApiCalled>>>> = Arc::new(Mutex::new(Vec::new()));
    let exc_buf: Arc<Mutex<Vec<Arc<EventExceptionThrown>>>> = Arc::new(Mutex::new(Vec::new()));
    let fail_buf: Arc<Mutex<Vec<Arc<EventLoadingFailed>>>> = Arc::new(Mutex::new(Vec::new()));
    let resp_buf: Arc<Mutex<Vec<Arc<EventResponseReceived>>>> = Arc::new(Mutex::new(Vec::new()));

    let c1 = console_buf.clone();
    let c2 = exc_buf.clone();
    let c3 = fail_buf.clone();
    let c4 = resp_buf.clone();
    let t1 = tokio::spawn(async move {
        while let Some(e) = console_stream.next().await {
            if let Ok(mut g) = c1.lock() {
                g.push(e);
            }
        }
    });
    let t2 = tokio::spawn(async move {
        while let Some(e) = exc_stream.next().await {
            if let Ok(mut g) = c2.lock() {
                g.push(e);
            }
        }
    });
    let t3 = tokio::spawn(async move {
        while let Some(e) = fail_stream.next().await {
            if let Ok(mut g) = c3.lock() {
                g.push(e);
            }
        }
    });
    let t4 = tokio::spawn(async move {
        while let Some(e) = resp_stream.next().await {
            if let Ok(mut g) = c4.lock() {
                g.push(e);
            }
        }
    });

    // Load the actual content.
    match input {
        RenderInput::Url(url) => {
            page.goto(url).await?;
            let _ = page.wait_for_navigation().await;
        }
        RenderInput::Html(html) => {
            page.set_content(html).await?;
        }
    }

    // Give JavaScript time to run.
    tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;

    // Snapshot metadata while the page is still alive.
    let title = page.get_title().await.ok().flatten().unwrap_or_default();
    let final_url = page.url().await.ok().flatten().unwrap_or_default();

    // Close the page; this also terminates the event streams, which lets the
    // collector tasks finish naturally.
    let _ = page.close().await;
    let _ = tokio::time::timeout(std::time::Duration::from_millis(200), async {
        let _ = t1.await;
        let _ = t2.await;
        let _ = t3.await;
        let _ = t4.await;
    })
    .await;

    let console: Vec<ConsoleMessage> = console_buf
        .lock()
        .map(|mut g| g.drain(..).map(|e| format_console_event(&e)).collect())
        .unwrap_or_default();
    let exceptions: Vec<JsException> = exc_buf
        .lock()
        .map(|mut g| g.drain(..).map(|e| format_exception(&e)).collect())
        .unwrap_or_default();
    let network_failures: Vec<NetFailure> = fail_buf
        .lock()
        .map(|mut g| g.drain(..).map(|e| format_failure(&e)).collect())
        .unwrap_or_default();
    let http_errors: Vec<HttpError> = resp_buf
        .lock()
        .map(|mut g| g.drain(..).filter_map(|e| format_http_error(&e)).collect())
        .unwrap_or_default();

    Ok(PageDiagnostics {
        title,
        final_url,
        elapsed_ms: start.elapsed().as_millis() as u64,
        console,
        exceptions,
        network_failures,
        http_errors,
    })
}

fn format_console_event(
    e: &chromiumoxide::cdp::js_protocol::runtime::EventConsoleApiCalled,
) -> ConsoleMessage {
    use chromiumoxide::cdp::js_protocol::runtime::ConsoleApiCalledType as Ty;
    let level = match &e.r#type {
        Ty::Log => "log",
        Ty::Debug => "debug",
        Ty::Info => "info",
        Ty::Warning => "warning",
        Ty::Error => "error",
        Ty::Trace => "trace",
        Ty::Assert => "assert",
        Ty::Dir => "dir",
        Ty::Dirxml => "dirxml",
        Ty::Table => "table",
        Ty::Clear => "clear",
        Ty::StartGroup => "startGroup",
        Ty::StartGroupCollapsed => "startGroupCollapsed",
        Ty::EndGroup => "endGroup",
        Ty::Profile => "profile",
        Ty::ProfileEnd => "profileEnd",
        Ty::Count => "count",
        Ty::TimeEnd => "timeEnd",
    }
    .to_string();
    let text = e
        .args
        .iter()
        .map(|arg| {
            if let Some(v) = arg.value.as_ref() {
                match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                }
            } else if let Some(d) = arg.description.as_ref() {
                d.clone()
            } else {
                format!("{:?}", arg.r#type)
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let (source_url, line) = e
        .stack_trace
        .as_ref()
        .and_then(|s| s.call_frames.first())
        .map(|f| (Some(f.url.clone()), Some(f.line_number as u32 + 1)))
        .unwrap_or((None, None));
    ConsoleMessage {
        level,
        text,
        source_url,
        line,
    }
}

fn format_exception(
    e: &chromiumoxide::cdp::js_protocol::runtime::EventExceptionThrown,
) -> JsException {
    let d = &e.exception_details;
    let text = d
        .exception
        .as_ref()
        .and_then(|r| r.description.clone())
        .or_else(|| {
            d.exception
                .as_ref()
                .and_then(|r| r.value.as_ref().map(|v| v.to_string()))
        })
        .unwrap_or_else(|| d.text.clone());
    let stack = d.stack_trace.as_ref().map(|st| {
        st.call_frames
            .iter()
            .map(|f| {
                format!(
                    "    at {} ({}:{}:{})",
                    if f.function_name.is_empty() {
                        "<anonymous>"
                    } else {
                        &f.function_name
                    },
                    f.url,
                    f.line_number + 1,
                    f.column_number + 1,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    });
    JsException {
        text,
        source_url: if d.url.is_some() { d.url.clone() } else { None },
        line: Some(d.line_number as u32 + 1),
        column: Some(d.column_number as u32 + 1),
        stack,
    }
}

fn format_failure(
    e: &chromiumoxide::cdp::browser_protocol::network::EventLoadingFailed,
) -> NetFailure {
    NetFailure {
        url: format!("(request {})", e.request_id.inner()),
        error_text: e.error_text.clone(),
        resource_type: format!("{:?}", e.r#type),
    }
}

fn format_http_error(
    e: &chromiumoxide::cdp::browser_protocol::network::EventResponseReceived,
) -> Option<HttpError> {
    let status = e.response.status as u16;
    if status < 400 {
        return None;
    }
    Some(HttpError {
        url: e.response.url.clone(),
        status,
        resource_type: format!("{:?}", e.r#type),
    })
}
