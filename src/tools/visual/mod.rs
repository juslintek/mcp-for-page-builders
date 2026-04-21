mod screenshot;
mod screenshot_page;
mod visual_compare;
mod extract_styles;
mod visual_diff;
mod match_styles;

pub use screenshot::Screenshot;
pub use screenshot_page::ScreenshotPage;
pub use visual_compare::VisualCompare;
pub use extract_styles::ExtractStyles;
pub use visual_diff::VisualDiff;
pub use match_styles::MatchStyles;

use anyhow::{Context, Result};
use chromiumoxide::page::ScreenshotParams;
use std::path::Path;

pub(crate) fn unix_timestamp() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// Takes a full-page screenshot and returns the raw PNG bytes + optional pre_js warning.
pub(crate) async fn cdp_screenshot(url: &str, output: &Path, width: u32, height: u32, pre_js: Option<&str>, wait_ms: u64) -> Result<(Vec<u8>, Option<String>)> {
    let (page, warning) = crate::cdp::open_page_with_js(url, width, height, pre_js, wait_ms).await?;
    let bytes = page.screenshot(ScreenshotParams::builder().full_page(true).build()).await.context("CDP screenshot failed")?;
    tokio::fs::write(output, &bytes).await.context("Failed to write screenshot")?;
    Ok((bytes, warning))
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
