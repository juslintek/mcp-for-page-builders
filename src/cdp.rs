//! Persistent CDP Chrome session shared across all visual tools.
//!
//! Chrome is launched lazily on first use via [`OnceCell`]. The handler task
//! runs in the background for the entire MCP server lifetime — Chrome stays alive.
//!
//! **Danger:** if Chrome crashes, the `OnceCell` is already initialized and will
//! not relaunch. Restart the MCP server to recover.
//!
//! `CHROME_PATH` env var overrides auto-detection of the Chrome executable.
//!
//! Each tool call creates a new [`Page`] (tab) — pages are **not** reused between calls.

use anyhow::{Context, Result};
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::Page;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::OnceCell;

static CDP: OnceCell<Arc<Browser>> = OnceCell::const_new();

/// Returns the singleton headless Chrome browser, launching it on first call.
///
/// Subsequent calls return the cached instance. The background handler task
/// keeps the browser alive until the process exits.
pub async fn browser() -> Result<Arc<Browser>> {
    CDP.get_or_try_init(|| async {
        let mut builder = BrowserConfig::builder()
            .arg("--disable-gpu")
            .arg("--no-sandbox")
            .arg("--disable-dev-shm-usage")
            .arg("--hide-scrollbars");

        if let Ok(path) = std::env::var("CHROME_PATH") {
            builder = builder.chrome_executable(path);
        }

        let config = builder.build().map_err(|e| anyhow::anyhow!("{e}"))?;
        let (browser, mut handler) = Browser::launch(config)
            .await
            .context("Failed to launch Chrome via CDP")?;

        tokio::spawn(async move {
            while handler.next().await.is_some() {}
        });

        tracing::info!("CDP Chrome session started");
        Ok(Arc::new(browser))
    })
    .await
    .cloned()
}

/// Opens a new browser tab, sets the viewport, navigates to `url`, and waits for load.
///
/// Each call produces an independent tab. Callers are responsible for closing
/// the page when done (or accepting the leak — tabs are cheap for short-lived tools).
pub async fn open_page(url: &str, width: u32, height: u32) -> Result<Page> {
    let b = browser().await?;
    let page = b.new_page("about:blank").await.context("Failed to create CDP page")?;

    if let Ok(cmd) = chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams::builder()
        .width(width)
        .height(height)
        .device_scale_factor(1.0)
        .mobile(false)
        .build()
    {
        let _ = page.execute(cmd).await;
    }

    page.goto(url).await.context("CDP navigation failed")?;
    page.wait_for_navigation().await.ok();
    Ok(page)
}
