use anyhow::Result;
use async_trait::async_trait;
use base64::Engine;
use chromiumoxide::page::ScreenshotParams;
use serde_json::{json, Value};
use std::path::Path;

use crate::args::{str_arg, u64_arg};
use crate::mcp::{ToolDef, ToolResult};
use crate::types::tool_result::ToolContent;
use crate::wp::WpClient;
use crate::tools::Tool;
use super::{cdp_screenshot, unix_timestamp, comparison_html};

pub struct VisualCompare;

#[async_trait]
impl Tool for VisualCompare {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "visual_compare",
            description: "Screenshot two URLs side-by-side. Returns a stitched comparison image (both pages in one image), plus individual images and an HTML file.\n\nWorkflow: Use to compare a reference design against your Elementor page. Use pre_js_a/pre_js_b to interact with each page before capture (e.g. expand menus on both).",
            input_schema: json!({"type":"object","required":["url_a","url_b"],"properties":{
                "url_a":{"type":"string"},
                "url_b":{"type":"string"},
                "label_a":{"type":"string","default":"A"},
                "label_b":{"type":"string","default":"B"},
                "output_dir":{"type":"string"},
                "width":{"type":"integer","default":1440},
                "height":{"type":"integer","default":900},
                "pre_js_a":{"type":"string","description":"JavaScript to execute on page A before screenshot"},
                "pre_js_b":{"type":"string","description":"JavaScript to execute on page B before screenshot"},
                "wait_ms":{"type":"integer","default":0,"description":"Milliseconds to wait after pre_js before capturing"}
            }}),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url_a = str_arg(&args, "url_a").ok_or_else(|| anyhow::anyhow!("url_a required"))?;
        let url_b = str_arg(&args, "url_b").ok_or_else(|| anyhow::anyhow!("url_b required"))?;
        let label_a = str_arg(&args, "label_a").unwrap_or_else(|| "A".into());
        let label_b = str_arg(&args, "label_b").unwrap_or_else(|| "B".into());
        let dir = str_arg(&args, "output_dir").unwrap_or_else(|| "/tmp".into());
        let width = u32::try_from(u64_arg(&args, "width").unwrap_or(1440)).unwrap_or(1440);
        let height = u32::try_from(u64_arg(&args, "height").unwrap_or(900)).unwrap_or(900);
        let pre_js_a = str_arg(&args, "pre_js_a");
        let pre_js_b = str_arg(&args, "pre_js_b");
        let wait_ms = u64_arg(&args, "wait_ms").unwrap_or(0);

        let t = unix_timestamp();
        let dir = Path::new(&dir);
        let img_a = dir.join(format!("compare-a-{t}.png"));
        let img_b = dir.join(format!("compare-b-{t}.png"));
        let html_out = dir.join(format!("compare-{t}.html"));
        let stitched_out = dir.join(format!("compare-stitched-{t}.png"));

        let (bytes_a, warn_a) = cdp_screenshot(&url_a, &img_a, width, height, pre_js_a.as_deref(), wait_ms).await?;
        let (bytes_b, warn_b) = cdp_screenshot(&url_b, &img_b, width, height, pre_js_b.as_deref(), wait_ms).await?;

        let html = comparison_html(&label_a, img_a.file_name().unwrap().to_str().unwrap(), &label_b, img_b.file_name().unwrap().to_str().unwrap(), &url_a, &url_b);
        tokio::fs::write(&html_out, &html).await?;

        // Generate stitched side-by-side image via CDP
        let b64_a = base64::engine::general_purpose::STANDARD.encode(&bytes_a);
        let b64_b = base64::engine::general_purpose::STANDARD.encode(&bytes_b);
        let stitch_html = format!(
            r#"data:text/html,<html><head><style>*{{margin:0;padding:0}}body{{display:flex;background:%23111}}.pane{{flex:1}}.label{{background:%231a1a2e;color:%23eee;font:600 14px system-ui;padding:8px 12px;text-align:center}}img{{width:100%;display:block}}</style></head><body><div class="pane"><div class="label">{label_a}</div><img src="data:image/png;base64,{b64_a}"></div><div class="pane"><div class="label">{label_b}</div><img src="data:image/png;base64,{b64_b}"></div></body></html>"#
        );

        let stitched_bytes = match stitch_via_cdp(&stitch_html, width * 2, height).await {
            Ok(b) => {
                let _ = tokio::fs::write(&stitched_out, &b).await;
                Some(b)
            }
            Err(e) => {
                tracing::warn!("Stitched image generation failed: {e}");
                None
            }
        };

        let mut text = format!("Comparison saved to {}\n{label_a}: {url_a}\n{label_b}: {url_b}", html_out.display());
        if let Some(w) = warn_a { text.push_str(&format!("\n⚠ {label_a}: {w}")); }
        if let Some(w) = warn_b { text.push_str(&format!("\n⚠ {label_b}: {w}")); }

        let mut content = vec![ToolContent::Text { text }];

        // Stitched image first (the main comparison view)
        if let Some(ref sb) = stitched_bytes {
            content.push(ToolContent::Image {
                data: base64::engine::general_purpose::STANDARD.encode(sb),
                mime_type: "image/png".into(),
            });
        }

        // Individual images as fallback
        content.push(ToolContent::Image { data: b64_a, mime_type: "image/png".into() });
        content.push(ToolContent::Image { data: b64_b, mime_type: "image/png".into() });

        Ok(ToolResult::mixed(content))
    }
}

async fn stitch_via_cdp(data_url: &str, width: u32, height: u32) -> Result<Vec<u8>> {
    let page = crate::cdp::open_page(data_url, width, height).await?;
    // Wait for images to render
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let bytes = page.screenshot(ScreenshotParams::builder().full_page(true).build()).await
        .map_err(|e| anyhow::anyhow!("Stitch screenshot failed: {e}"))?;
    Ok(bytes)
}
