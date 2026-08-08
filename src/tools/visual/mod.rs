mod screenshot;
mod screenshot_page;
mod visual_compare;
mod extract_styles;
mod ui_quality_audit;
mod visual_diff;
mod match_styles;

pub use screenshot::Screenshot;
pub use screenshot_page::ScreenshotPage;
pub use visual_compare::VisualCompare;
pub use extract_styles::ExtractStyles;
pub use ui_quality_audit::UiQualityAudit;
pub use visual_diff::VisualDiff;
pub use match_styles::MatchStyles;

use anyhow::{Context, Result};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::ScreenshotParams;
use std::path::Path;
use std::time::Duration;

/// Inline image size budget (bytes of base64). A single JSON-RPC response line
/// is read into a fixed buffer by most MCP clients. Kiro CLI uses a ~1 MB line
/// buffer; at 4/3 base64 expansion a 700 KB JPEG inline image fits with room to
/// spare, while large full-page PNGs (up to 2–3 MB) were blowing the buffer and
/// causing the client to report "Transport closed / non-JSON-RPC output" errors.
pub(crate) const MAX_INLINE_JPEG_BYTES: usize = 700 * 1024; // 700 KB

/// JPEG quality used when the PNG is within budget. Lowered progressively if the
/// first encode still exceeds the budget.
const JPEG_QUALITY: i64 = 82;

/// Bound each screenshot capture so a hung CDP call fails fast instead of
/// blocking the MCP request indefinitely.
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn unix_timestamp() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Takes a full-page screenshot.
///
/// * Saves the full-quality PNG to `output` on disk (unchanged behaviour).
/// * Returns a JPEG-encoded copy for the inline AI response to keep the
///   JSON-RPC line well under MCP client read-buffer limits.
///   If JPEG encoding fails the PNG bytes are returned as a fallback.
pub(crate) async fn cdp_screenshot(
    url: &str,
    output: &Path,
    width: u32,
    height: u32,
    pre_js: Option<&str>,
    wait_ms: u64,
) -> Result<(Vec<u8>, Option<String>)> {
    cdp_screenshot_opts(url, output, width, height, pre_js, wait_ms, false).await
}

pub(crate) async fn cdp_screenshot_opts(
    url: &str,
    output: &Path,
    width: u32,
    height: u32,
    pre_js: Option<&str>,
    wait_ms: u64,
    reuse_tab: bool,
) -> Result<(Vec<u8>, Option<String>)> {
    let (page, warning, _reused) = crate::cdp::open_page_with_js_opts(url, width, height, pre_js, wait_ms, reuse_tab).await?;

    // Always capture full-quality PNG for disk storage. Bounded by a timeout so
    // a hung capture cannot block the MCP request forever.
    let png_bytes = match tokio::time::timeout(
        SCREENSHOT_TIMEOUT,
        page.screenshot(ScreenshotParams::builder().full_page(true).build()),
    )
    .await
    {
        Ok(r) => r.context("CDP screenshot failed")?,
        Err(_) => anyhow::bail!("CDP screenshot timed out after {}s", SCREENSHOT_TIMEOUT.as_secs()),
    };

    tokio::fs::write(output, &png_bytes)
        .await
        .context("Failed to write screenshot")?;

    // Capture a JPEG copy for the inline response — much smaller than PNG.
    // Try quality 82 first; if still over budget drop to 60, then 40.
    let inline_bytes = 'jpeg: {
        for quality in [JPEG_QUALITY, 60, 40] {
            let shot = tokio::time::timeout(
                SCREENSHOT_TIMEOUT,
                page.screenshot(
                    ScreenshotParams::builder()
                        .full_page(true)
                        .format(CaptureScreenshotFormat::Jpeg)
                        .quality(quality)
                        .build(),
                ),
            )
            .await;
            match shot {
                Ok(Ok(jpeg)) if jpeg.len() <= MAX_INLINE_JPEG_BYTES => break 'jpeg jpeg,
                Ok(Ok(jpeg)) => {
                    tracing::debug!(
                        "JPEG at quality {quality} is {}KB — trying lower quality",
                        jpeg.len() / 1024
                    );
                }
                Ok(Err(e)) => {
                    tracing::warn!("JPEG capture failed at quality {quality}: {e:#}");
                    break 'jpeg png_bytes.clone();
                }
                Err(_) => {
                    tracing::warn!("JPEG capture timed out at quality {quality}");
                    break 'jpeg png_bytes.clone();
                }
            }
        }
        // Last resort: return the PNG (better than nothing; may still be large).
        tracing::warn!(
            "Could not produce a JPEG under {}KB; falling back to PNG for inline response",
            MAX_INLINE_JPEG_BYTES / 1024
        );
        png_bytes.clone()
    };

    if !reuse_tab {
        let _ = page.close().await;
    }
    Ok((inline_bytes, warning))
}


pub(crate) fn comparison_html(label_a: &str, img_a: &str, label_b: &str, img_b: &str, url_a: &str, url_b: &str) -> String {
    format!(r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{label_a} vs {label_b}</title>
<style>*{{box-sizing:border-box;margin:0;padding:0}}body{{font-family:system-ui;background:#111;color:#eee}}header{{padding:12px 20px;background:#222;display:flex;gap:20px;align-items:center}}header h1{{font-size:14px}}.url{{font-size:11px;color:#888}}.grid{{display:grid;grid-template-columns:1fr 1fr;height:calc(100vh - 48px)}}.pane{{overflow:auto;border-right:1px solid #333}}.pane:last-child{{border-right:none}}.pane-header{{position:sticky;top:0;background:#1a1a2e;padding:8px 12px;font-size:12px;font-weight:600;z-index:1;border-bottom:1px solid #333}}.pane img{{width:100%;display:block}}</style>
</head><body>
<header><h1>Visual Comparison</h1><span class="url">{label_a}: {url_a}</span><span class="url">{label_b}: {url_b}</span></header>
<div class="grid"><div class="pane"><div class="pane-header">{label_a}</div><img src="{img_a}"></div><div class="pane"><div class="pane-header">{label_b}</div><img src="{img_b}"></div></div>
</body></html>"#)
}
