use anyhow::{Context, Result};
use async_trait::async_trait;
use chromiumoxide::page::ScreenshotParams;
use serde_json::{json, Value};
use std::path::Path;

use crate::args::{str_arg, u64_arg};
use crate::cdp;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Opens a CDP page at `url`, captures a full-page PNG screenshot, and writes it to `output`.
///
/// Used by [`Screenshot`], [`ScreenshotPage`], and [`VisualCompare`] to avoid duplicating
/// the open → screenshot → write sequence.
async fn cdp_screenshot(url: &str, output: &Path, width: u32, height: u32) -> Result<()> {
    let page = cdp::open_page(url, width, height).await?;
    let bytes = page
        .screenshot(ScreenshotParams::builder().full_page(true).build())
        .await
        .context("CDP screenshot failed")?;
    tokio::fs::write(output, &bytes).await.context("Failed to write screenshot")?;
    Ok(())
}

/// Captures a full-page PNG screenshot of any publicly accessible URL.
///
/// Uses headless Chrome via CDP. The output path defaults to `/tmp/screenshot-{timestamp}.png`.
/// Width and height set the viewport — the screenshot captures the full scrollable page.
pub struct Screenshot;

#[async_trait]
impl Tool for Screenshot {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "screenshot",
            description: "Capture a full-page screenshot of any URL using headless Chrome. Saves to a PNG file.",
            input_schema: json!({
                "type": "object",
                "required": ["url"],
                "properties": {
                    "url": { "type": "string" },
                    "output": { "type": "string", "description": "Output PNG path. Defaults to /tmp/screenshot-{timestamp}.png" },
                    "width": { "type": "integer", "default": 1440 },
                    "height": { "type": "integer", "default": 900 }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url = str_arg(&args, "url").ok_or_else(|| anyhow::anyhow!("url required"))?;
        let output = str_arg(&args, "output").unwrap_or_else(|| format!("/tmp/screenshot-{}.png", unix_timestamp()));
        let width = u64_arg(&args, "width").unwrap_or(1440) as u32;
        let height = u64_arg(&args, "height").unwrap_or(900) as u32;

        cdp_screenshot(&url, Path::new(&output), width, height).await?;
        Ok(ToolResult::text(format!("Screenshot saved to {output}")))
    }
}

/// Screenshots a WordPress page identified by its post ID.
///
/// Resolves the page's public URL via the REST API, then delegates to [`cdp_screenshot`].
/// Accepts an optional `query` string appended to the URL (e.g. `force_elementor=1`).
pub struct ScreenshotPage;

#[async_trait]
impl Tool for ScreenshotPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "screenshot_page",
            description: "Screenshot a WordPress page by ID. Optionally append query params (e.g. '?force_elementor=1').",
            input_schema: json!({
                "type": "object",
                "required": ["page_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "query": { "type": "string", "description": "Query string to append (e.g. 'force_elementor=1')" },
                    "output": { "type": "string", "description": "Output PNG path" },
                    "width": { "type": "integer", "default": 1440 },
                    "height": { "type": "integer", "default": 900 }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let query = str_arg(&args, "query").unwrap_or_default();
        let width = u64_arg(&args, "width").unwrap_or(1440) as u32;
        let height = u64_arg(&args, "height").unwrap_or(900) as u32;

        let page = wp.get(&format!("wp/v2/pages/{page_id}")).await?;
        let link = page["link"].as_str().ok_or_else(|| anyhow::anyhow!("Page has no link"))?;
        let url = if query.is_empty() { link.to_string() } else { format!("{link}?{query}") };

        let output = str_arg(&args, "output").unwrap_or_else(|| format!("/tmp/page-{page_id}-{}.png", unix_timestamp()));
        cdp_screenshot(&url, Path::new(&output), width, height).await?;
        Ok(ToolResult::text(format!("Screenshot of page {page_id} saved to {output}\nURL: {url}")))
    }
}

/// Screenshots two URLs side-by-side and generates a self-contained HTML comparison file.
///
/// Both screenshots are captured sequentially via [`cdp_screenshot`] and saved to `output_dir`.
/// The HTML file embeds relative `<img>` references — open it in a browser from the same directory.
///
/// Useful for comparing a legacy page against its Elementor rebuild.
pub struct VisualCompare;

#[async_trait]
impl Tool for VisualCompare {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "visual_compare",
            description: "Screenshot two URLs side-by-side and generate an HTML comparison file. Perfect for comparing legacy vs Elementor versions of a page.",
            input_schema: json!({
                "type": "object",
                "required": ["url_a", "url_b"],
                "properties": {
                    "url_a": { "type": "string", "description": "Left side URL (e.g. legacy page)" },
                    "url_b": { "type": "string", "description": "Right side URL (e.g. Elementor page)" },
                    "label_a": { "type": "string", "default": "A" },
                    "label_b": { "type": "string", "default": "B" },
                    "output_dir": { "type": "string", "description": "Directory for output files. Defaults to /tmp" },
                    "width": { "type": "integer", "default": 1440 },
                    "height": { "type": "integer", "default": 900 }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url_a = str_arg(&args, "url_a").ok_or_else(|| anyhow::anyhow!("url_a required"))?;
        let url_b = str_arg(&args, "url_b").ok_or_else(|| anyhow::anyhow!("url_b required"))?;
        let label_a = str_arg(&args, "label_a").unwrap_or_else(|| "A".into());
        let label_b = str_arg(&args, "label_b").unwrap_or_else(|| "B".into());
        let dir = str_arg(&args, "output_dir").unwrap_or_else(|| "/tmp".into());
        let width = u64_arg(&args, "width").unwrap_or(1440) as u32;
        let height = u64_arg(&args, "height").unwrap_or(900) as u32;

        let t = unix_timestamp();
        let dir = Path::new(&dir);
        let img_a = dir.join(format!("compare-a-{t}.png"));
        let img_b = dir.join(format!("compare-b-{t}.png"));
        let html_out = dir.join(format!("compare-{t}.html"));

        cdp_screenshot(&url_a, &img_a, width, height).await?;
        cdp_screenshot(&url_b, &img_b, width, height).await?;

        let html = comparison_html(
            &label_a, img_a.file_name().unwrap().to_str().unwrap(),
            &label_b, img_b.file_name().unwrap().to_str().unwrap(),
            &url_a, &url_b,
        );
        tokio::fs::write(&html_out, &html).await?;

        Ok(ToolResult::text(format!(
            "Comparison saved to {}\nOpen in browser: open {}",
            html_out.display(), html_out.display()
        )))
    }
}

/// Extracts computed CSS properties from a live DOM element via CDP.
///
/// Returns a flat JSON object of property→value pairs plus `_rect`, `_tag`, `_classes`,
/// and `_childCount` metadata. Useful for replicating legacy designs in Elementor without
/// manually inspecting DevTools.
///
/// The default property list covers layout, typography, colors, and positioning.
/// Override with the `properties` parameter to narrow or expand the set.
pub struct ExtractStyles;

#[async_trait]
impl Tool for ExtractStyles {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "extract_styles",
            description: "Extract computed CSS properties from a live page element using headless Chrome. Returns key layout/visual properties as JSON — useful for replicating legacy designs in Elementor.",
            input_schema: json!({
                "type": "object",
                "required": ["url", "selector"],
                "properties": {
                    "url": { "type": "string", "description": "Page URL to inspect" },
                    "selector": { "type": "string", "description": "CSS selector for the element, e.g. 'header', '.site-header', '#main'" },
                    "properties": { "type": "array", "items": {"type": "string"}, "description": "Specific CSS properties to extract. Defaults to a comprehensive layout/visual set." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url = str_arg(&args, "url").ok_or_else(|| anyhow::anyhow!("url required"))?;
        let selector = str_arg(&args, "selector").ok_or_else(|| anyhow::anyhow!("selector required"))?;

        let custom_props: Option<Vec<String>> = args.get("properties")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());

        let props_js = custom_props
            .unwrap_or_else(|| DEFAULT_PROPS.iter().map(|s| s.to_string()).collect())
            .iter()
            .map(|p| format!("'{p}'"))
            .collect::<Vec<_>>()
            .join(",");

        let js = format!(
            r#"(()=>{{const el=document.querySelector('{selector}');if(!el)return JSON.stringify({{error:'Element not found: {selector}'}});const s=getComputedStyle(el);const r={{}};[{props_js}].forEach(p=>r[p]=s.getPropertyValue(p));const b=el.getBoundingClientRect();r._rect={{x:Math.round(b.x),y:Math.round(b.y),w:Math.round(b.width),h:Math.round(b.height)}};r._tag=el.tagName;r._classes=el.className?.substring?.(0,200)||'';r._childCount=el.children.length;return JSON.stringify(r)}})()"#
        );

        let page = cdp::open_page(&url, 1440, 900).await?;
        let result: String = page.evaluate(js).await
            .context("CDP evaluate failed")?
            .into_value()
            .context("Failed to get JS result")?;

        let val: Value = serde_json::from_str(&result).context("Failed to parse styles JSON")?;
        if let Some(err) = val.get("error") {
            anyhow::bail!("Style extraction failed: {}", err.as_str().unwrap_or("unknown"));
        }
        Ok(ToolResult::text(serde_json::to_string_pretty(&val)?))
    }
}

const DEFAULT_PROPS: &[&str] = &[
    "width", "height", "min-height", "max-width",
    "padding", "margin",
    "background-color", "color",
    "font-size", "font-weight", "font-family",
    "display", "flex-direction", "justify-content", "align-items", "gap",
    "position", "z-index", "top", "left",
    "border", "border-radius", "box-shadow", "opacity",
];

/// Compares two pages element-by-element via CDP and returns structured, actionable differences.
///
/// This is a **structural diff**, not a pixel diff — it compares computed CSS values and
/// bounding boxes for each selector on both pages.
///
/// Returns a `match_score` percentage (0–100) indicating overall similarity, plus a
/// `differences` array where each entry has a `severity` of `"high"`, `"medium"`, or `"low"`:
/// - `high` — element missing on one side, or dimension differs by >20px.
/// - `medium` — CSS property value differs, or dimension differs by 5–20px.
/// - `low` — text content differs.
pub struct VisualDiff;

#[async_trait]
impl Tool for VisualDiff {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "visual_diff",
            description: "Compare two pages element-by-element via CDP and return structured, actionable differences (not pixel diff).",
            input_schema: json!({
                "type": "object",
                "required": ["url_a", "url_b", "selectors"],
                "properties": {
                    "url_a": { "type": "string" },
                    "url_b": { "type": "string" },
                    "selectors": { "type": "array", "items": {"type": "string"}, "description": "CSS selectors to compare on both pages" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url_a = str_arg(&args, "url_a").ok_or_else(|| anyhow::anyhow!("url_a required"))?;
        let url_b = str_arg(&args, "url_b").ok_or_else(|| anyhow::anyhow!("url_b required"))?;
        let selectors: Vec<String> = args.get("selectors")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        if selectors.is_empty() {
            anyhow::bail!("selectors array required");
        }

        let build_element_snapshot_js = |sels: &[String]| -> String {
            let sels_js = sels.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",");
            format!(
                r#"(()=>{{const results={{}};[{sels_js}].forEach(sel=>{{const el=document.querySelector(sel);if(!el){{results[sel]=null;return;}}const s=getComputedStyle(el);const b=el.getBoundingClientRect();results[sel]={{box:{{x:Math.round(b.x),y:Math.round(b.y),w:Math.round(b.width),h:Math.round(b.height)}},styles:{{'background-color':s.backgroundColor,color:s.color,'font-size':s.fontSize,'font-weight':s.fontWeight,padding:s.padding,margin:s.margin,'border-radius':s.borderRadius,display:s.display,'flex-direction':s.flexDirection,gap:s.gap,opacity:s.opacity}},text:el.textContent?.trim()?.substring(0,100)||''}}}});return JSON.stringify(results)}})()
"#
            )
        };

        let page_a = cdp::open_page(&url_a, 1440, 900).await?;
        let result_a: String = page_a.evaluate(build_element_snapshot_js(&selectors)).await?.into_value()?;
        let data_a: Value = serde_json::from_str(&result_a)?;

        let page_b = cdp::open_page(&url_b, 1440, 900).await?;
        let result_b: String = page_b.evaluate(build_element_snapshot_js(&selectors)).await?.into_value()?;
        let data_b: Value = serde_json::from_str(&result_b)?;

        let mut diffs = Vec::new();
        let mut matched = 0u32;
        let mut total = 0u32;

        for sel in &selectors {
            let a = data_a.get(sel);
            let b = data_b.get(sel);
            match (a, b) {
                (Some(Value::Null) | None, Some(Value::Null) | None) => {}
                (Some(_), Some(Value::Null) | None) => {
                    diffs.push(json!({"selector": sel, "issue": "missing on B", "severity": "high"}));
                    total += 1;
                }
                (Some(Value::Null) | None, Some(_)) => {
                    diffs.push(json!({"selector": sel, "issue": "missing on A", "severity": "high"}));
                    total += 1;
                }
                (Some(va), Some(vb)) => {
                    if let (Some(ba), Some(bb)) = (va.get("box"), vb.get("box")) {
                        for dim in ["w", "h"] {
                            total += 1;
                            let da = ba.get(dim).and_then(|v| v.as_i64()).unwrap_or(0);
                            let db = bb.get(dim).and_then(|v| v.as_i64()).unwrap_or(0);
                            if (da - db).abs() > 5 {
                                let label = if dim == "w" { "width" } else { "height" };
                                diffs.push(json!({"selector": sel, "property": label, "value_a": da, "value_b": db, "severity": if (da-db).abs() > 20 {"high"} else {"medium"}}));
                            } else {
                                matched += 1;
                            }
                        }
                    }
                    if let (Some(sa), Some(sb)) = (va.get("styles"), vb.get("styles")) {
                        if let (Some(oa), Some(ob)) = (sa.as_object(), sb.as_object()) {
                            for (k, va_val) in oa {
                                total += 1;
                                let vb_val = ob.get(k).unwrap_or(&Value::Null);
                                if va_val == vb_val {
                                    matched += 1;
                                } else {
                                    diffs.push(json!({"selector": sel, "property": k, "value_a": va_val, "value_b": vb_val, "severity": "medium"}));
                                }
                            }
                        }
                    }
                    let ta = va.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    let tb = vb.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    if !ta.is_empty() || !tb.is_empty() {
                        total += 1;
                        if ta == tb { matched += 1; } else {
                            diffs.push(json!({"selector": sel, "property": "text", "value_a": ta, "value_b": tb, "severity": "low"}));
                        }
                    }
                }
                _ => {}
            }
        }

        let score = if total > 0 { (matched as f64 / total as f64 * 100.0) as u32 } else { 100 };
        let output = json!({ "match_score": score, "total_checks": total, "differences": diffs });
        Ok(ToolResult::text(serde_json::to_string_pretty(&output)?))
    }
}

fn comparison_html(label_a: &str, img_a: &str, label_b: &str, img_b: &str, url_a: &str, url_b: &str) -> String {
    format!(r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{label_a} vs {label_b}</title>
<style>*{{box-sizing:border-box;margin:0;padding:0}}body{{font-family:system-ui;background:#111;color:#eee}}header{{padding:12px 20px;background:#222;display:flex;gap:20px;align-items:center}}header h1{{font-size:14px}}.url{{font-size:11px;color:#888}}.grid{{display:grid;grid-template-columns:1fr 1fr;height:calc(100vh - 48px)}}.pane{{overflow:auto;border-right:1px solid #333}}.pane:last-child{{border-right:none}}.pane-header{{position:sticky;top:0;background:#1a1a2e;padding:8px 12px;font-size:12px;font-weight:600;z-index:1;border-bottom:1px solid #333}}.pane img{{width:100%;display:block}}</style>
</head><body>
<header><h1>Visual Comparison</h1><span class="url">{label_a}: {url_a}</span><span class="url">{label_b}: {url_b}</span></header>
<div class="grid"><div class="pane"><div class="pane-header">{label_a}</div><img src="{img_a}"></div><div class="pane"><div class="pane-header">{label_b}</div><img src="{img_b}"></div></div>
</body></html>"#)
}
