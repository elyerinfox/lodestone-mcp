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
