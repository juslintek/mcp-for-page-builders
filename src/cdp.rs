//! Persistent CDP Chrome session with automatic crash recovery.
//!
//! Chrome is launched lazily on first use. If Chrome crashes, the next call
//! detects the failure, resets the session, and relaunches automatically.
//!
//! `CHROME_PATH` env var overrides auto-detection of the Chrome executable.
//!
//! Each tool call creates a new [`Page`] (tab) — pages are **not** reused between calls.

use anyhow::{Context, Result};
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::Page;
use futures::StreamExt;
use std::sync::Arc;
use tokio::sync::RwLock;

static CDP: RwLock<Option<Arc<Browser>>> = RwLock::const_new(None);

async fn launch_browser() -> Result<Arc<Browser>> {
    let user_data_dir = std::env::temp_dir().join("mcp-for-page-builders-cdp");
    let _ = std::fs::create_dir_all(&user_data_dir);

    // Kill any orphaned Chrome using our data dir, then clean stale lock
    #[cfg(unix)]
    {
        let dir_str = user_data_dir.display().to_string();
        let _ = std::process::Command::new("pkill")
            .args(["-f", &format!("user-data-dir=.*{dir_str}")])
            .output();
        // Give the process time to die
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let lock = user_data_dir.join("SingletonLock");
    if lock.exists() {
        let _ = std::fs::remove_file(&lock);
        tracing::info!("Removed stale SingletonLock");
    }

    let mut builder = BrowserConfig::builder()
        .user_data_dir(user_data_dir)
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
}

/// Returns the shared headless Chrome browser, launching or relaunching as needed.
pub async fn browser() -> Result<Arc<Browser>> {
    // Fast path: already initialized
    {
        let guard = CDP.read().await;
        if let Some(b) = guard.as_ref() {
            return Ok(Arc::clone(b));
        }
    }
    // Slow path: launch
    let mut guard = CDP.write().await;
    // Double-check after acquiring write lock
    if let Some(b) = guard.as_ref() {
        return Ok(Arc::clone(b));
    }
    let b = launch_browser().await?;
    *guard = Some(Arc::clone(&b));
    Ok(b)
}

/// Reset the browser session. Next call to `browser()` will relaunch.
pub async fn reset() {
    let mut guard = CDP.write().await;
    *guard = None;
    tracing::info!("CDP Chrome session reset");
}

/// Opens a new browser tab, sets the viewport, navigates to `url`, and waits for load.
pub async fn open_page(url: &str, width: u32, height: u32) -> Result<Page> {
    let (page, _) = open_page_with_js(url, width, height, None, 0).await?;
    Ok(page)
}

/// Opens a page with optional JS execution after load. Returns (Page, Option<warning>).
pub async fn open_page_with_js(url: &str, width: u32, height: u32, pre_js: Option<&str>, wait_ms: u64) -> Result<(Page, Option<String>)> {
    match try_open_page(url, width, height, pre_js, wait_ms).await {
        Ok(result) => Ok(result),
        Err(first_err) => {
            tracing::warn!("CDP page open failed, resetting Chrome: {first_err:#}");
            reset().await;
            try_open_page(url, width, height, pre_js, wait_ms).await.context("CDP retry after reset also failed")
        }
    }
}

async fn try_open_page(url: &str, width: u32, height: u32, pre_js: Option<&str>, wait_ms: u64) -> Result<(Page, Option<String>)> {
    // Pre-navigation reachability check for local URLs
    let (env_type, _) = crate::tools::ensure_site::detect_env(url);
    if !matches!(env_type, crate::tools::ensure_site::EnvType::Remote) {
        if crate::tools::ensure_site::check_reachable(url).await.is_err() {
            anyhow::bail!("Site unreachable: {url}\nThis looks like a {env_type} environment. Call ensure_site first to boot it.");
        }
    }

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

    let mut warning: Option<String> = None;

    if let Some(js) = pre_js {
        // Capture state before pre_js
        let before: String = page.evaluate("'' + document.body.scrollHeight + '|' + document.body.innerHTML.length").await
            .ok().and_then(|v| v.into_value().ok()).unwrap_or_default();

        page.evaluate(js).await.ok();
        if wait_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
        }

        // Check if pre_js had any effect
        let after: String = page.evaluate("'' + document.body.scrollHeight + '|' + document.body.innerHTML.length").await
            .ok().and_then(|v| v.into_value().ok()).unwrap_or_default();

        if before == after && !before.is_empty() {
            let msg = "pre_js executed but page state unchanged — the selector may not exist on this site".to_string();
            tracing::warn!("{msg}");
            warning = Some(msg);
        }
    }

    Ok((page, warning))
}
