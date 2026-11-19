mod extract_styles;
mod match_styles;
mod screenshot;
mod screenshot_page;
mod ui_quality_audit;
mod visual_compare;
mod visual_diff;

pub use extract_styles::ExtractStyles;
pub use match_styles::MatchStyles;
pub use screenshot::Screenshot;
pub use screenshot_page::ScreenshotPage;
pub use ui_quality_audit::UiQualityAudit;
pub use visual_compare::VisualCompare;
pub use visual_diff::VisualDiff;

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
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(15);
const CLOSE_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Takes a full-page screenshot.
///
/// * Saves the image to `output` on disk.
/// * Returns a JPEG-encoded copy for the inline AI response to keep the
///   JSON-RPC line well under MCP client read-buffer limits.
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
    let (page, warning, _reused) =
        crate::cdp::open_page_with_js_opts(url, width, height, pre_js, wait_ms, reuse_tab).await?;

    // Capture JPEG directly — single CDP call, fast & low overhead.
    let jpeg_res = tokio::time::timeout(
        SCREENSHOT_TIMEOUT,
        page.screenshot(
            ScreenshotParams::builder()
                .full_page(true)
                .format(CaptureScreenshotFormat::Jpeg)
                .quality(JPEG_QUALITY)
                .build(),
        ),
    )
    .await;

    let image_bytes = match jpeg_res {
        Ok(Ok(jpeg)) if jpeg.len() <= MAX_INLINE_JPEG_BYTES => jpeg,
        Ok(Ok(jpeg)) => {
            // Quality 82 exceeded budget; try quality 50 once
            tracing::debug!(
                "JPEG at quality {JPEG_QUALITY} is {}KB — trying quality 50",
                jpeg.len() / 1024
            );
            let lower_res = tokio::time::timeout(
                SCREENSHOT_TIMEOUT,
                page.screenshot(
                    ScreenshotParams::builder()
                        .full_page(true)
                        .format(CaptureScreenshotFormat::Jpeg)
                        .quality(50)
                        .build(),
                ),
            )
            .await;
            match lower_res {
                Ok(Ok(jpeg50)) => jpeg50,
                _ => jpeg,
            }
        }
        _ => {
            // Fallback to PNG if JPEG capture failed
            tracing::warn!("JPEG screenshot failed or timed out, trying PNG fallback");
            tokio::time::timeout(
                SCREENSHOT_TIMEOUT,
                page.screenshot(ScreenshotParams::builder().full_page(true).build()),
            )
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "CDP screenshot timed out after {}s",
                    SCREENSHOT_TIMEOUT.as_secs()
                )
            })?
            .context("CDP screenshot failed")?
        }
    };

    tokio::fs::write(output, &image_bytes)
        .await
        .context("Failed to write screenshot")?;

    if !reuse_tab {
        let _ = tokio::time::timeout(CLOSE_TIMEOUT, page.close()).await;
    }
    Ok((image_bytes, warning))
}

pub(crate) fn comparison_html(
    label_a: &str,
    img_a: &str,
    label_b: &str,
    img_b: &str,
    url_a: &str,
    url_b: &str,
) -> String {
    format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{label_a} vs {label_b}</title>
<style>*{{box-sizing:border-box;margin:0;padding:0}}body{{font-family:system-ui;background:#111;color:#eee}}header{{padding:12px 20px;background:#222;display:flex;gap:20px;align-items:center}}header h1{{font-size:14px}}.url{{font-size:11px;color:#888}}.grid{{display:grid;grid-template-columns:1fr 1fr;height:calc(100vh - 48px)}}.pane{{overflow:auto;border-right:1px solid #333}}.pane:last-child{{border-right:none}}.pane-header{{position:sticky;top:0;background:#1a1a2e;padding:8px 12px;font-size:12px;font-weight:600;z-index:1;border-bottom:1px solid #333}}.pane img{{width:100%;display:block}}</style>
</head><body>
<header><h1>Visual Comparison</h1><span class="url">{label_a}: {url_a}</span><span class="url">{label_b}: {url_b}</span></header>
<div class="grid"><div class="pane"><div class="pane-header">{label_a}</div><img src="{img_a}"></div><div class="pane"><div class="pane-header">{label_b}</div><img src="{img_b}"></div></div>
</body></html>"#
    )
}

/// Maximum pixel dimension (width or height) for images inlined in the MCP
/// response. Model providers (Anthropic) reject images exceeding 8000px on
/// either axis. We cap at 7680 to leave a small margin.
const MAX_INLINE_DIMENSION: u32 = 7680;

/// Downscale JPEG bytes if either dimension exceeds `MAX_INLINE_DIMENSION`.
/// Returns the original bytes unchanged if already within limits.
pub(crate) fn cap_image_for_inline(jpeg_bytes: &[u8]) -> Vec<u8> {
    use image::ImageReader;
    use std::io::Cursor;

    let reader = match ImageReader::new(Cursor::new(jpeg_bytes)).with_guessed_format() {
        Ok(r) => r,
        Err(_) => return jpeg_bytes.to_vec(),
    };
    let img = match reader.decode() {
        Ok(i) => i,
        Err(_) => return jpeg_bytes.to_vec(),
    };

    let (w, h) = (img.width(), img.height());
    if w <= MAX_INLINE_DIMENSION && h <= MAX_INLINE_DIMENSION {
        return jpeg_bytes.to_vec();
    }

    // Scale to fit within MAX_INLINE_DIMENSION on the longest axis
    let scale = f64::from(MAX_INLINE_DIMENSION) / f64::from(w.max(h));
    let new_w = (f64::from(w) * scale) as u32;
    let new_h = (f64::from(h) * scale) as u32;

    tracing::debug!("Downscaling inline image from {w}x{h} to {new_w}x{new_h}");
    let resized = img.resize(new_w, new_h, image::imageops::FilterType::Triangle);

    let mut buf = Cursor::new(Vec::new());
    let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 70);
    match resized.write_with_encoder(encoder) {
        Ok(()) => buf.into_inner(),
        Err(_) => jpeg_bytes.to_vec(),
    }
}
