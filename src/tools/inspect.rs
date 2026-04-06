use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::cdp;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

/// Inspects a live DOM element via CDP `Runtime.evaluate`.
///
/// Returns a structured tree with bounding boxes, computed styles, and children
/// up to `max_depth` levels deep. No screenshot is needed — data is purely structural.
///
/// **Danger:** uses `querySelector` which returns only the **first** matching element.
/// Deep trees with large `max_depth` can produce very large JSON payloads.
///
/// The default CSS property list covers layout, typography, colors, and positioning.
/// Override with the `properties` parameter to narrow or expand the set.
pub struct InspectPage;

#[async_trait]
impl Tool for InspectPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "inspect_page",
            description: "Inspect a DOM element via CDP — returns bounding box, computed styles, and children tree. Structured data, no screenshot needed.",
            input_schema: json!({
                "type": "object",
                "required": ["url", "selector"],
                "properties": {
                    "url": { "type": "string" },
                    "selector": { "type": "string", "description": "CSS selector, e.g. 'header', '.site-header'" },
                    "max_depth": { "type": "integer", "default": 3, "description": "Max recursion depth for children" },
                    "properties": { "type": "array", "items": {"type": "string"}, "description": "CSS properties to extract (defaults to comprehensive set)" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let url = args["url"].as_str().ok_or_else(|| anyhow::anyhow!("url required"))?;
        let selector = args["selector"].as_str().ok_or_else(|| anyhow::anyhow!("selector required"))?;
        let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(3);

        let custom_props: Option<Vec<String>> = args.get("properties")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect());

        let props_js = custom_props
            .unwrap_or_else(|| DEFAULT_PROPS.iter().map(|s| s.to_string()).collect())
            .iter()
            .map(|p| format!("'{p}'"))
            .collect::<Vec<_>>()
            .join(",");

        let js = build_inspect_js(selector, &props_js, max_depth);

        let page = cdp::open_page(url, 1440, 900).await?;
        let result: String = page.evaluate(js).await
            .context("CDP evaluate failed")?
            .into_value()
            .context("Failed to get JS result")?;

        let val: Value = serde_json::from_str(&result).context("Failed to parse inspect JSON")?;
        if let Some(err) = val.get("error") {
            anyhow::bail!("{}", err.as_str().unwrap_or("unknown error"));
        }
        Ok(ToolResult::text(serde_json::to_string_pretty(&val)?))
    }
}

/// Builds the self-contained JS snippet that walks the DOM from `selector` up to `max_depth`.
///
/// `props_js` is a comma-separated list of quoted CSS property names, e.g. `'color','font-size'`.
fn build_inspect_js(selector: &str, props_js: &str, max_depth: u64) -> String {
    format!(
        r#"(()=>{{function inspect(el,depth){{if(!el||depth<0)return null;const s=getComputedStyle(el);const b=el.getBoundingClientRect();const styles={{}};[{props_js}].forEach(p=>styles[p]=s.getPropertyValue(p));const children=[];if(depth>0){{for(const c of el.children){{const r=inspect(c,depth-1);if(r)children.push(r)}}}}const cls=el.className&&el.className.substring?el.className.substring(0,200):'';const tag=el.tagName.toLowerCase();const id=el.id?'#'+el.id:'';const clsStr=cls?'.'+cls.split(/\s+/).join('.'):'';return{{element:tag+id+clsStr,tag:el.tagName,box:{{x:Math.round(b.x),y:Math.round(b.y),w:Math.round(b.width),h:Math.round(b.height)}},styles,text:el.childNodes.length===1&&el.childNodes[0].nodeType===3?el.textContent.trim().substring(0,200):'',children}}}}const el=document.querySelector('{selector}');if(!el)return JSON.stringify({{error:'Element not found: {selector}'}});return JSON.stringify(inspect(el,{max_depth}))}})()"#
    )
}

const DEFAULT_PROPS: &[&str] = &[
    "width", "height", "min-height", "max-width",
    "padding", "padding-top", "padding-right", "padding-bottom", "padding-left",
    "margin", "margin-top", "margin-right", "margin-bottom", "margin-left",
    "background-color", "background-image", "color",
    "font-size", "font-weight", "font-family", "line-height", "text-transform", "letter-spacing",
    "display", "flex-direction", "justify-content", "align-items", "gap",
    "position", "z-index", "top", "left", "right", "bottom",
    "border", "border-radius", "box-shadow", "opacity", "overflow",
];
