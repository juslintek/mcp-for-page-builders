//! Persistent CDP Chrome session with automatic crash recovery.
//!
//! Chrome is launched lazily on first use. If Chrome crashes, the next call
//! detects the failure, resets the session, and relaunches automatically.
//!
//! `CHROME_PATH` env var overrides auto-detection of the Chrome executable.
//!
//! Each tool call creates a new [`Page`] (tab) — pages are **not** reused between calls.

use anyhow::{Context, Result};
use chromiumoxide::Page;
use chromiumoxide::browser::{Browser, BrowserConfig};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabInfo {
    pub url: String,
    pub target_id: String,
}

struct CdpSession {
    browser: Arc<Browser>,
    profile: std::path::PathBuf,
}

static CDP: RwLock<Option<CdpSession>> = RwLock::const_new(None);
static CDP_PROFILE_GENERATION: AtomicU64 = AtomicU64::new(0);

// Bounded CDP operation timeouts. Without these, a hung Chrome launch,
// navigation, or in-page script blocks the MCP request forever — the client
// then appears to "crash"/freeze and must be cancelled. Failing fast lets the
// caller (and the open_page_with_js reset+retry path) recover cleanly.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
const NAV_TIMEOUT: Duration = Duration::from_secs(20);

/// Query all open tabs in the current Chrome browser session.
pub async fn list_tabs() -> Result<Vec<TabInfo>> {
    let b = browser().await?;
    let pages = tokio::time::timeout(Duration::from_secs(5), b.pages())
        .await
        .context("CDP list pages timed out")?
        .context("Failed to list browser pages")?;
    let mut tabs = Vec::with_capacity(pages.len());
    for page in &pages {
        let url = tokio::time::timeout(Duration::from_secs(2), page.url())
            .await
            .ok()
            .and_then(Result::ok)
            .flatten()
            .unwrap_or_default();
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
    let pages = tokio::time::timeout(Duration::from_secs(5), b.pages())
        .await
        .context("CDP list pages timed out")?
        .context("Failed to list browser pages")?;
    if pages.is_empty() {
        return Ok(None);
    }

    if !url.is_empty() {
        for page in &pages {
            if let Ok(Ok(Some(page_url))) =
                tokio::time::timeout(Duration::from_secs(2), page.url()).await
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

fn cdp_profile_dir(temp_dir: &std::path::Path, pid: u32, generation: u64) -> std::path::PathBuf {
    temp_dir.join(format!("mcp-for-page-builders-cdp-{pid}-{generation}"))
}

fn create_cdp_profile(
    temp_dir: &std::path::Path,
    pid: u32,
    generation: &AtomicU64,
) -> Result<std::path::PathBuf> {
    for _ in 0..64 {
        let path = cdp_profile_dir(temp_dir, pid, generation.fetch_add(1, Ordering::Relaxed));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to create private CDP profile: {}", path.display())
                });
            }
        }
    }
    anyhow::bail!("Could not allocate a fresh CDP profile after 64 collisions")
}

fn remove_cdp_profile(path: &std::path::Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            anyhow::bail!(
                "Refusing to remove non-directory CDP profile: {}",
                path.display()
            );
        }
        Ok(_) => std::fs::remove_dir_all(path)
            .with_context(|| format!("Failed to remove CDP profile: {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Failed to inspect CDP profile before removal"),
    }
}

async fn launch_browser() -> Result<CdpSession> {
    let profile = create_cdp_profile(
        &std::env::temp_dir(),
        std::process::id(),
        &CDP_PROFILE_GENERATION,
    )?;

    let mut builder = BrowserConfig::builder().user_data_dir(&profile);

    if let Ok(path) = std::env::var("CHROME_PATH") {
        builder = builder.chrome_executable(path);
    }

    let config = match builder.build() {
        Ok(config) => config,
        Err(error) => {
            remove_cdp_profile(&profile)?;
            return Err(anyhow::anyhow!("{error}"));
        }
    };
    let (browser, mut handler) = match tokio::time::timeout(LAUNCH_TIMEOUT, Browser::launch(config))
        .await
    {
        Ok(Ok(browser)) => browser,
        Ok(Err(error)) => {
            remove_cdp_profile(&profile)?;
            return Err(error).context("Failed to launch Chrome via CDP");
        }
        Err(_) => anyhow::bail!(
            "Chrome launch timed out after {}s (profile preserved at {} because process shutdown is uncertain)",
            LAUNCH_TIMEOUT.as_secs(),
            profile.display()
        ),
    };

    tokio::spawn(async move { while handler.next().await.is_some() {} });

    tracing::info!("CDP Chrome session started");
    Ok(CdpSession {
        browser: Arc::new(browser),
        profile,
    })
}

/// Returns the shared headless Chrome browser, launching or relaunching as needed.
pub async fn browser() -> Result<Arc<Browser>> {
    // Fast path: already initialized
    {
        let guard = CDP.read().await;
        if let Some(b) = guard.as_ref() {
            return Ok(Arc::clone(&b.browser));
        }
    }
    // Slow path: launch
    let mut guard = CDP.write().await;
    // Double-check after acquiring write lock
    if let Some(b) = guard.as_ref() {
        return Ok(Arc::clone(&b.browser));
    }
    let session = launch_browser().await?;
    let browser = Arc::clone(&session.browser);
    *guard = Some(session);
    Ok(browser)
}

async fn stop_owned_browser(mut browser: Browser, profile: std::path::PathBuf) {
    let closed = tokio::time::timeout(Duration::from_secs(3), browser.close())
        .await
        .is_ok_and(|result| result.is_ok());
    let stopped = if closed {
        tokio::time::timeout(Duration::from_secs(3), browser.wait())
            .await
            .is_ok_and(|result| result.is_ok_and(|status| status.is_some()))
    } else {
        false
    };
    let stopped = if stopped {
        true
    } else {
        tokio::time::timeout(Duration::from_secs(3), browser.kill())
            .await
            .is_ok_and(|result| result.is_some_and(|result| result.is_ok()))
    };

    if stopped {
        if let Err(error) = remove_cdp_profile(&profile) {
            tracing::warn!("Failed to remove owned CDP profile: {error:#}");
        }
    } else {
        tracing::warn!(
            "Could not confirm owned CDP browser shutdown; preserving its profile: {}",
            profile.display()
        );
    }
}

/// Reset the browser session. Next call to `browser()` will relaunch.
pub async fn reset() {
    let mut guard = CDP.write().await;
    if let Some(session) = guard.take() {
        match Arc::try_unwrap(session.browser) {
            Ok(browser) => stop_owned_browser(browser, session.profile).await,
            Err(browser) => {
                tokio::spawn(async move {
                    while Arc::strong_count(&browser) > 1 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    match Arc::try_unwrap(browser) {
                        Ok(browser) => stop_owned_browser(browser, session.profile).await,
                        Err(_) => tracing::warn!("CDP Browser ownership changed during reset"),
                    }
                });
            }
        }
    }
    tracing::info!("CDP Chrome session reset");
}

/// Opens a new browser tab, sets the viewport, navigates to `url`, and waits for load.
pub async fn open_page(url: &str, width: u32, height: u32) -> Result<Page> {
    let (page, _) = open_page_with_js(url, width, height, None, 0).await?;
    Ok(page)
}

/// Opens a page with optional JS execution after load. Returns (Page, Option<warning>).
pub async fn open_page_with_js(
    url: &str,
    width: u32,
    height: u32,
    pre_js: Option<&str>,
    wait_ms: u64,
) -> Result<(Page, Option<String>)> {
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
        && crate::tools::ensure_site::check_reachable(url)
            .await
            .is_err()
    {
        anyhow::bail!(
            "Site unreachable: {url}\nThis looks like a {env_type} environment. Call ensure_site first to boot it."
        );
    }

    let eval_timeout = Duration::from_secs(5);

    if reuse_existing && let Ok(Some(page)) = find_matching_page(url).await {
        let _ = tokio::time::timeout(Duration::from_secs(2), page.bring_to_front()).await;

        if let Ok(cmd) = chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams::builder()
            .width(width)
            .height(height)
            .device_scale_factor(1.0)
            .mobile(false)
            .build()
        {
            let _ = tokio::time::timeout(Duration::from_secs(2), page.execute(cmd)).await;
        }

        let mut warning: Option<String> = None;

        if let Some(js) = pre_js {
            // Capture state before pre_js
            let before: String = tokio::time::timeout(
                eval_timeout,
                page.evaluate(
                    "'' + document.body.scrollHeight + '|' + document.body.innerHTML.length",
                ),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|v| v.into_value().ok())
            .unwrap_or_default();

            let _ = tokio::time::timeout(NAV_TIMEOUT, page.evaluate(js)).await;
            if wait_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
            }

            // Check if pre_js had any effect
            let after: String = tokio::time::timeout(
                eval_timeout,
                page.evaluate(
                    "'' + document.body.scrollHeight + '|' + document.body.innerHTML.length",
                ),
            )
            .await
            .ok()
            .and_then(Result::ok)
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
    let page = match tokio::time::timeout(Duration::from_secs(10), b.new_page("about:blank")).await
    {
        Ok(r) => r.context("Failed to create CDP page")?,
        Err(_) => anyhow::bail!("CDP new_page timed out"),
    };

    if let Ok(cmd) =
        chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams::builder()
            .width(width)
            .height(height)
            .device_scale_factor(1.0)
            .mobile(false)
            .build()
    {
        let _ = tokio::time::timeout(Duration::from_secs(2), page.execute(cmd)).await;
    }

    match tokio::time::timeout(NAV_TIMEOUT, page.goto(url)).await {
        Ok(r) => {
            r.context("CDP navigation failed")?;
        }
        Err(_) => anyhow::bail!(
            "CDP navigation to {url} timed out after {}s",
            NAV_TIMEOUT.as_secs()
        ),
    }

    let mut warning: Option<String> = None;

    if let Some(js) = pre_js {
        // Capture state before pre_js
        let before: String = tokio::time::timeout(
            eval_timeout,
            page.evaluate("'' + document.body.scrollHeight + '|' + document.body.innerHTML.length"),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .and_then(|v| v.into_value().ok())
        .unwrap_or_default();

        let _ = tokio::time::timeout(NAV_TIMEOUT, page.evaluate(js)).await;
        if wait_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
        }

        // Check if pre_js had any effect
        let after: String = tokio::time::timeout(
            eval_timeout,
            page.evaluate("'' + document.body.scrollHeight + '|' + document.body.innerHTML.length"),
        )
        .await
        .ok()
        .and_then(Result::ok)
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
        assert!(urls_match(
            "http://localhost:8080/path/",
            "http://localhost:8080/path"
        ));
        assert!(!urls_match(
            "https://example.com/a",
            "https://example.com/b"
        ));
    }

    #[test]
    fn profile_cleanup_is_scoped_and_refuses_symlinks() {
        let root = std::env::temp_dir().join(format!(
            "mcp-for-page-builders-cdp-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();

        let generation = AtomicU64::new(0);
        let collision = cdp_profile_dir(&root, 101, 0);
        let sentinel = cdp_profile_dir(&root, 202, 99);
        let target = root.join("target");
        std::fs::create_dir(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &collision).unwrap();
        #[cfg(not(unix))]
        std::fs::create_dir(&collision).unwrap();
        std::fs::create_dir(&sentinel).unwrap();

        let owned = create_cdp_profile(&root, 101, &generation).unwrap();
        assert_eq!(owned, cdp_profile_dir(&root, 101, 1));
        assert!(collision.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&owned).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        remove_cdp_profile(&owned).unwrap();
        assert!(!owned.exists());
        assert!(sentinel.is_dir());

        #[cfg(unix)]
        {
            assert!(remove_cdp_profile(&collision).is_err());
            assert!(target.is_dir());
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn reset_relaunches_while_old_browser_is_shared() {
        let sentinel = std::env::temp_dir().join(format!(
            "mcp-for-page-builders-cdp-sentinel-{}-{}",
            std::process::id(),
            CDP_PROFILE_GENERATION.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&sentinel).unwrap();

        let first = browser().await.unwrap();
        let page = first.new_page("about:blank").await.unwrap();
        assert_eq!(page.url().await.unwrap().as_deref(), Some("about:blank"));
        drop(page);
        let first_profile = CDP.read().await.as_ref().unwrap().profile.clone();

        reset().await;
        assert!(first_profile.is_dir());

        let second = browser().await.unwrap();
        let second_profile = CDP.read().await.as_ref().unwrap().profile.clone();
        assert_ne!(first_profile, second_profile);
        let page = second.new_page("about:blank").await.unwrap();
        assert_eq!(page.url().await.unwrap().as_deref(), Some("about:blank"));
        drop(page);
        drop(second);

        reset().await;
        assert!(!second_profile.exists());
        assert!(sentinel.is_dir());

        drop(first);
        for _ in 0..100 {
            if !first_profile.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(!first_profile.exists());
        assert!(sentinel.is_dir());
        std::fs::remove_dir(sentinel).unwrap();
    }
}
