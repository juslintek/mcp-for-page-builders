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
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub url: String,
    pub target_id: String,
}

static CDP: RwLock<Option<Arc<Browser>>> = RwLock::const_new(None);

// Bounded CDP operation timeouts. Without these, a hung Chrome launch,
// navigation, or in-page script blocks the MCP request forever — the client
// then appears to "crash"/freeze and must be cancelled. Failing fast lets the
// caller (and the open_page_with_js reset+retry path) recover cleanly.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
const NAV_TIMEOUT: Duration = Duration::from_secs(20);

/// Query all open tabs in the current Chrome browser session.
pub async fn list_tabs() -> Result<Vec<TabInfo>> {
    let b = browser().await?;
    let pages = b.pages().await.context("Failed to list browser pages")?;
    let mut tabs = Vec::with_capacity(pages.len());
    for page in &pages {
        let url = page.url().await.ok().flatten().unwrap_or_default();
        let target_id = page.target_id().inner().clone();
        tabs.push(TabInfo { url, target_id });
    }
    Ok(tabs)
}

fn urls_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let norm_a = a.trim_end_matches('/');
    let norm_b = b.trim_end_matches('/');
    !norm_a.is_empty() && norm_a == norm_b
}

/// Look up an open tab matching `url` or active tab.
pub async fn find_matching_page(url: &str) -> Result<Option<Page>> {
    let b = browser().await?;
    let pages = b.pages().await.context("Failed to list browser pages")?;
    if pages.is_empty() {
        return Ok(None);
    }

    if !url.is_empty() {
        for page in &pages {
            if let Ok(Some(page_url)) = page.url().await
                && urls_match(&page_url, url)
            {
                return Ok(Some(page.clone()));
            }
        }
    } else if let Some(active) = pages.last() {
        return Ok(Some(active.clone()));
    }

    Ok(None)
}

/// Best-effort cleanup of profile directories (and any Chrome process still
/// using them) left behind by a previous `mcp-for-page-builders` instance
/// that crashed or was killed without a chance to clean up after itself.
/// Only removes dirs whose PID suffix no longer corresponds to a live
/// process — never touches a dir belonging to another currently-running
/// instance. Safe to call on every startup; failures are non-fatal.
#[cfg(unix)]
fn sweep_stale_cdp_profiles() {
    let temp_dir = std::env::temp_dir();
    let Ok(entries) = std::fs::read_dir(&temp_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(pid_str) = name.strip_prefix("mcp-for-page-builders-cdp-") else {
            continue;
        };
        let Ok(pid) = pid_str.parse::<u32>() else {
            continue;
        };
        if pid == std::process::id() {
            continue; // never touch our own dir
        }
        // `kill -0` checks liveness without sending a real signal.
        let alive = std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .is_ok_and(|o| o.status.success());
        if alive {
            continue; // that instance is still running, leave it alone
        }
        let dir_str = entry.path().display().to_string();
        let _ = std::process::Command::new("pkill")
            .args(["-f", &format!("user-data-dir=.*{dir_str}")])
            .output();
        let _ = std::fs::remove_dir_all(entry.path());
        tracing::info!("Swept stale CDP profile from dead PID {pid}: {dir_str}");
    }
}

async fn launch_browser() -> Result<Arc<Browser>> {
    #[cfg(unix)]
    sweep_stale_cdp_profiles();

    // Scope the Chrome profile dir to this process's PID. Multiple
    // mcp-for-page-builders instances can run concurrently (e.g. parallel
    // subagent sessions each spawning their own MCP server) — a shared,
    // fixed profile dir caused them to race: one instance's stale-lock
    // cleanup / pkill would kill a *different* instance's legitimate Chrome
    // process, surfacing as "transport closed" / non-JSON-RPC stdout errors
    // in whichever MCP client lost the race. A PID-scoped dir makes every
    // server instance fully independent; the sweep above reaps directories
    // left behind by crashed instances so they don't accumulate forever.
    let user_data_dir = std::env::temp_dir().join(format!(
        "mcp-for-page-builders-cdp-{}",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&user_data_dir);

    // Best-effort cleanup of a stale lock from a previous *crashed* run of
    // this exact PID's dir (extremely unlikely given PIDs aren't reused
    // quickly, but harmless if it does happen). No pkill here — this
    // process's own dir can only ever be touched by itself.
    let lock = user_data_dir.join("SingletonLock");
    if lock.exists() {
        let _ = std::fs::remove_file(&lock);
        tracing::info!("Removed stale SingletonLock");
    }

    // Note: --no-sandbox, --disable-dev-shm-usage, --hide-scrollbars, --disable-gpu
    // are already added by chromiumoxide's DEFAULT_ARGS / headless mode — do NOT
    // add them again via .arg() or they appear doubled (e.g. ----hide-scrollbars)
    // in the Chrome command line, which while harmless today can cause unexpected
    // behaviour in future Chrome versions.
    let mut builder = BrowserConfig::builder()
        .user_data_dir(user_data_dir);

    if let Ok(path) = std::env::var("CHROME_PATH") {
        builder = builder.chrome_executable(path);
    }

    let config = builder.build().map_err(|e| anyhow::anyhow!("{e}"))?;
    let (browser, mut handler) = match tokio::time::timeout(LAUNCH_TIMEOUT, Browser::launch(config)).await {
        Ok(r) => r.context("Failed to launch Chrome via CDP")?,
        Err(_) => anyhow::bail!(
            "Chrome launch timed out after {}s (is Chrome installed? try setting CHROME_PATH)",
            LAUNCH_TIMEOUT.as_secs()
        ),
    };

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
    open_page_with_js_opts(url, width, height, pre_js, wait_ms, false)
        .await
        .map(|(p, w, _)| (p, w))
}

/// Opens a page (or reuses an existing tab) with optional JS execution.
/// Returns `(Page, Option<warning>, reused_existing)`.
pub async fn open_page_with_js_opts(
    url: &str,
    width: u32,
    height: u32,
    pre_js: Option<&str>,
    wait_ms: u64,
    reuse_existing: bool,
) -> Result<(Page, Option<String>, bool)> {
    match try_open_page_opts(url, width, height, pre_js, wait_ms, reuse_existing).await {
        Ok(result) => Ok(result),
        Err(first_err) => {
            tracing::warn!("CDP page open failed, resetting Chrome: {first_err:#}");
            reset().await;
            try_open_page_opts(url, width, height, pre_js, wait_ms, reuse_existing)
                .await
                .context("CDP retry after reset also failed")
        }
    }
}

async fn try_open_page_opts(
    url: &str,
    width: u32,
    height: u32,
    pre_js: Option<&str>,
    wait_ms: u64,
    reuse_existing: bool,
) -> Result<(Page, Option<String>, bool)> {
    // Pre-navigation reachability check for local URLs
    let (env_type, _) = crate::tools::ensure_site::detect_env(url);
    if !matches!(env_type, crate::tools::ensure_site::EnvType::Remote)
        && crate::tools::ensure_site::check_reachable(url).await.is_err()
    {
        anyhow::bail!("Site unreachable: {url}\nThis looks like a {env_type} environment. Call ensure_site first to boot it.");
    }

    if reuse_existing
        && let Ok(Some(page)) = find_matching_page(url).await
    {
        let _ = page.bring_to_front().await;

            if let Ok(cmd) = chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams::builder()
                .width(width)
                .height(height)
                .device_scale_factor(1.0)
                .mobile(false)
                .build()
            {
                let _ = page.execute(cmd).await;
            }

            let mut warning: Option<String> = None;

            if let Some(js) = pre_js {
                // Capture state before pre_js
                let before: String = page
                    .evaluate("'' + document.body.scrollHeight + '|' + document.body.innerHTML.length")
                    .await
                    .ok()
                    .and_then(|v| v.into_value().ok())
                    .unwrap_or_default();

                let _ = tokio::time::timeout(NAV_TIMEOUT, page.evaluate(js)).await;
                if wait_ms > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
                }

                // Check if pre_js had any effect
                let after: String = page
                    .evaluate("'' + document.body.scrollHeight + '|' + document.body.innerHTML.length")
                    .await
                    .ok()
                    .and_then(|v| v.into_value().ok())
                    .unwrap_or_default();

                if before == after && !before.is_empty() {
                    let msg = "pre_js executed but page state unchanged — the selector may not exist on this site".to_string();
                    tracing::warn!("{msg}");
                    warning = Some(msg);
                }
            }

            return Ok((page, warning, true));
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

    match tokio::time::timeout(NAV_TIMEOUT, page.goto(url)).await {
        Ok(r) => {
            r.context("CDP navigation failed")?;
        }
        Err(_) => anyhow::bail!("CDP navigation to {url} timed out after {}s", NAV_TIMEOUT.as_secs()),
    }
    // Best-effort: many SPA/canvas pages never fire a second navigation event,
    // so bound this and ignore the result rather than blocking indefinitely.
    let _ = tokio::time::timeout(NAV_TIMEOUT, page.wait_for_navigation()).await;

    let mut warning: Option<String> = None;

    if let Some(js) = pre_js {
        // Capture state before pre_js
        let before: String = page
            .evaluate("'' + document.body.scrollHeight + '|' + document.body.innerHTML.length")
            .await
            .ok()
            .and_then(|v| v.into_value().ok())
            .unwrap_or_default();

        let _ = tokio::time::timeout(NAV_TIMEOUT, page.evaluate(js)).await;
        if wait_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
        }

        // Check if pre_js had any effect
        let after: String = page
            .evaluate("'' + document.body.scrollHeight + '|' + document.body.innerHTML.length")
            .await
            .ok()
            .and_then(|v| v.into_value().ok())
            .unwrap_or_default();

        if before == after && !before.is_empty() {
            let msg = "pre_js executed but page state unchanged — the selector may not exist on this site".to_string();
            tracing::warn!("{msg}");
            warning = Some(msg);
        }
    }

    Ok((page, warning, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_info_serde() {
        let tab = TabInfo {
            url: "https://example.com".to_string(),
            target_id: "ABCD1234".to_string(),
        };
        let json = serde_json::to_string(&tab).unwrap();
        let deserialized: TabInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.url, "https://example.com");
        assert_eq!(deserialized.target_id, "ABCD1234");
    }

    #[test]
    fn test_urls_match() {
        assert!(urls_match("https://example.com/", "https://example.com"));
        assert!(urls_match("https://example.com", "https://example.com/"));
        assert!(urls_match("http://localhost:8080/path/", "http://localhost:8080/path"));
        assert!(!urls_match("https://example.com/a", "https://example.com/b"));
    }
}


