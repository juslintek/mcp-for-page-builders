use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::cdp;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

const DEFAULT_PROPS: &[&str] = &[
    "width", "height", "min-height", "max-width",
    "padding", "margin",
    "background-color", "color",
    "font-size", "font-weight", "font-family",
    "display", "flex-direction", "justify-content", "align-items", "gap",
    "position", "z-index", "top", "left",
    "border", "border-radius", "box-shadow", "opacity",
];

pub struct ExtractStyles;

#[async_trait]
impl Tool for ExtractStyles {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "extract_styles",
            description: "Extract computed CSS properties from a live page element using headless Chrome.",
            input_schema: json!({"type":"object","required":["url","selector"],"properties":{"url":{"type":"string"},"selector":{"type":"string"},"properties":{"type":"array","items":{"type":"string"}}}}),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url = str_arg(&args, "url").ok_or_else(|| anyhow::anyhow!("url required"))?;
        let selector = str_arg(&args, "selector").ok_or_else(|| anyhow::anyhow!("selector required"))?;

        let custom_props: Option<Vec<String>> = args.get("properties")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());

        let props_js = custom_props
            .unwrap_or_else(|| DEFAULT_PROPS.iter().map(std::string::ToString::to_string).collect())
            .iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(",");

        let js = format!(
            r"(()=>{{const el=document.querySelector('{selector}');if(!el)return JSON.stringify({{error:'Element not found: {selector}'}});const s=getComputedStyle(el);const r={{}};[{props_js}].forEach(p=>r[p]=s.getPropertyValue(p));const b=el.getBoundingClientRect();r._rect={{x:Math.round(b.x),y:Math.round(b.y),w:Math.round(b.width),h:Math.round(b.height)}};r._tag=el.tagName;r._classes=el.className?.substring?.(0,200)||'';r._childCount=el.children.length;return JSON.stringify(r)}})()"
        );

        let page = cdp::open_page(&url, 1440, 900).await?;
        let result: String = page.evaluate(js).await.context("CDP evaluate failed")?.into_value().context("Failed to get JS result")?;

        let val: Value = serde_json::from_str(&result).context("Failed to parse styles JSON")?;
        if let Some(err) = val.get("error") {
            anyhow::bail!("Style extraction failed: {}", err.as_str().unwrap_or("unknown"));
        }
        Ok(ToolResult::text(serde_json::to_string_pretty(&val)?))
    }
}
