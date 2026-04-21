use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::args::{str_arg, u64_arg};
use crate::cdp;
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::css_map::map_css_to_elementor;
use crate::wp::WpClient;
use crate::tools::Tool;

const EXTRACT_JS_PROPS: &[&str] = &[
    "background-color", "color", "font-size", "font-weight", "font-family",
    "line-height", "letter-spacing", "text-transform",
    "padding", "padding-top", "padding-right", "padding-bottom", "padding-left",
    "margin", "margin-top", "margin-right", "margin-bottom", "margin-left",
    "border", "border-radius", "gap", "display", "flex-direction",
    "justify-content", "align-items", "width", "height", "min-height",
    "max-width", "opacity", "box-shadow", "overflow",
];

pub struct MatchStyles;

#[async_trait]
impl Tool for MatchStyles {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "match_styles",
            description: "One-shot visual parity: extract computed styles from a reference URL element, convert to Elementor settings, and apply to a target element.\n\nWorkflow: The recommended entry point for style matching. Combines extract_styles + css_to_elementor + update_element in one call. Set verify=true to also run visual_diff afterward.",
            input_schema: json!({
                "type": "object",
                "required": ["reference_url", "reference_selector", "page_id", "element_id"],
                "properties": {
                    "reference_url": { "type": "string", "description": "URL of the reference page to extract styles from" },
                    "reference_selector": { "type": "string", "description": "CSS selector of the reference element" },
                    "page_id": { "type": "integer", "description": "Target Elementor page ID" },
                    "element_id": { "type": "string", "description": "Target Elementor element ID (8-char)" },
                    "verify": { "type": "boolean", "default": false, "description": "Run visual_diff after applying to check results" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let ref_url = str_arg(&args, "reference_url").ok_or_else(|| anyhow::anyhow!("reference_url required"))?;
        let ref_sel = str_arg(&args, "reference_selector").ok_or_else(|| anyhow::anyhow!("reference_selector required"))?;
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid = str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;
        let verify = args.get("verify").and_then(|v| v.as_bool()).unwrap_or(false);

        // 1. Extract computed styles from reference
        let props_js = EXTRACT_JS_PROPS.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(",");
        let js = format!(
            r"(()=>{{const el=document.querySelector('{ref_sel}');if(!el)return JSON.stringify({{error:'not found: {ref_sel}'}});const s=getComputedStyle(el);const r={{}};[{props_js}].forEach(p=>{{const v=s.getPropertyValue(p);if(v&&v!=='none'&&v!=='normal'&&v!=='auto'&&v!=='0px'&&v!=='rgba(0, 0, 0, 0)')r[p]=v}});return JSON.stringify(r)}})()"
        );

        let page = cdp::open_page(&ref_url, 1440, 900).await?;
        let result: String = page.evaluate(js).await.context("CDP evaluate failed")?.into_value()?;
        let css_val: Value = serde_json::from_str(&result)?;
        if let Some(err) = css_val.get("error") {
            anyhow::bail!("{}", err.as_str().unwrap_or("extraction failed"));
        }
        let css_map = css_val.as_object().ok_or_else(|| anyhow::anyhow!("Expected CSS object"))?;

        // 2. Convert CSS to Elementor settings
        let (settings, unmapped) = map_css_to_elementor(css_map);

        if settings.is_empty() {
            return Ok(ToolResult::text("No mappable CSS properties found on the reference element."));
        }

        // 3. Apply to target element
        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;
        let new_settings = Value::Object(settings.clone());
        let found = tree.mutate(&eid, |el| crate::elementor::merge_settings(&mut el.settings, &new_settings));
        if !found {
            return Ok(ToolResult::error(format!("Element {eid} not found on page {page_id}")));
        }
        svc.save_tree(page_id, &tree).await?;

        // 4. Build result
        let mut output = Map::new();
        output.insert("applied_settings".into(), json!(settings));
        output.insert("settings_count".into(), json!(settings.len()));
        if !unmapped.is_empty() {
            output.insert("unmapped_css".into(), json!(unmapped));
        }

        // 5. Optional verification via visual_diff
        if verify {
            let target_page = wp.get(&format!("wp/v2/pages/{page_id}")).await?;
            if let Some(link) = target_page["link"].as_str() {
                output.insert("verify_hint".into(), json!(format!(
                    "Run visual_diff with url_a={ref_url} url_b={link} to compare results"
                )));
            }
        }

        Ok(ToolResult::text(serde_json::to_string_pretty(&Value::Object(output))?))
    }
}
