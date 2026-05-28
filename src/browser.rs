//! Headless-browser rendering as its own capability (feature `browser`).
//!
//! A [`PageRenderer`] renders a URL with a real browser — executing JavaScript —
//! and returns the resulting HTML. The Chromium implementation keeps a single
//! persistent browser alive (launching one per request adds ~10s of latency)
//! and is exposed process-wide via [`shared_global`], so the Google provider,
//! the StackOverflow scraper, and on-demand `fetch_page`/search rendering all
//! reuse the same browser.

use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use tokio::sync::Mutex;

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
#[derive(Clone, Default)]
pub struct BrowserOptions {
    /// Path to the Chrome/Chromium executable; empty = auto-detect.
    pub chrome_path: String,
    /// Pass `--no-sandbox` and `--disable-dev-shm-usage`. Required when running
    /// Chrome as root (e.g. inside a Docker container).
    pub no_sandbox: bool,
    /// Additional command-line flags to pass to Chrome.
    pub args: Vec<String>,
}

/// A [`PageRenderer`] backed by a persistent headless Chrome/Chromium instance.
/// Concurrent renders serialize on the single browser via an async mutex.
pub struct ChromiumRenderer {
    options: BrowserOptions,
    browser: Mutex<Option<BrowserHandle>>,
}

impl ChromiumRenderer {
    pub fn new(options: BrowserOptions) -> Self {
        Self {
            options,
            browser: Mutex::new(None),
        }
    }
}

#[async_trait]
impl PageRenderer for ChromiumRenderer {
    async fn render(&self, url: &str) -> Result<String> {
        let mut guard = self.browser.lock().await;
        if guard.is_none() {
            *guard = Some(launch(&self.options).await?);
        }
        match render_page(guard.as_mut().unwrap(), url).await {
            Ok(html) => Ok(html),
            Err(e) => {
                // The browser may have died; relaunch once and retry.
                tracing::warn!(error = %e, "headless browser failed; relaunching");
                *guard = Some(launch(&self.options).await?);
                render_page(guard.as_mut().unwrap(), url).await
            }
        }
    }

    async fn render_pdf(&self, url: &str) -> Result<Vec<u8>> {
        let mut guard = self.browser.lock().await;
        if guard.is_none() {
            *guard = Some(launch(&self.options).await?);
        }
        match page_to_pdf(guard.as_mut().unwrap(), url).await {
            Ok(bytes) => Ok(bytes),
            Err(e) => {
                tracing::warn!(error = %e, "headless browser failed; relaunching");
                *guard = Some(launch(&self.options).await?);
                page_to_pdf(guard.as_mut().unwrap(), url).await
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
    let (browser, mut handler) = Browser::launch(config).await?;
    let driver = tokio::spawn(async move { while handler.next().await.is_some() {} });
    Ok(BrowserHandle {
        browser,
        _driver: driver,
    })
}

async fn render_page(handle: &mut BrowserHandle, url: &str) -> Result<String> {
    let page = handle.browser.new_page(url).await?;
    page.wait_for_navigation().await?;
    let html = page.content().await?;
    let _ = page.close().await;
    Ok(html)
}

async fn page_to_pdf(handle: &mut BrowserHandle, url: &str) -> Result<Vec<u8>> {
    use chromiumoxide::cdp::browser_protocol::page::PrintToPdfParams;
    let page = handle.browser.new_page(url).await?;
    page.wait_for_navigation().await?;
    let bytes = page.pdf(PrintToPdfParams::default()).await?;
    let _ = page.close().await;
    Ok(bytes)
}
