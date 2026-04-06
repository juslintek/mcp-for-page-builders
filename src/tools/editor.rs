use anyhow::{Context, Result};
use async_trait::async_trait;
use chromiumoxide::Page;
use serde_json::{json, Value};

use crate::cdp;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

/// Controls the Elementor editor via CDP by evaluating JS against Elementor's internal API (`$e.run`).
///
/// **Danger:** `$e.run` is an undocumented internal API that may change between Elementor versions.
///
/// Requires an authenticated WordPress session — the CDP browser must already be logged in.
/// The editor must be opened first (`action: 'open'`) before any other action will work.
///
/// Pages are **not** closed after `open` — the editor tab persists and is reused by subsequent
/// actions via [`active_editor_page`], which always picks the last open tab.
pub struct ElementorEditor;

/// Returns the last open CDP page, which is expected to be the Elementor editor tab.
///
/// All editor actions after `open` use this to avoid re-opening the browser.
async fn active_editor_page() -> Result<Page> {
    let b = cdp::browser().await?;
    let pages = b.pages().await.context("No pages")?;
    pages.into_iter().last().ok_or_else(|| anyhow::anyhow!("No editor page open"))
}

fn js_select_widget(element_id: &str) -> String {
    format!(
        r#"(()=>{{const container=elementor.getContainer('{element_id}');if(!container)return 'not_found';$e.run('panel/editor/open',{{model:container.model,view:container.view}});return 'selected'}})()"#
    )
}

fn js_set_setting(element_id: &str, key: &str, val_js: &str) -> String {
    format!(
        r#"(()=>{{const container=elementor.getContainer('{element_id}');if(!container)return 'not_found';$e.run('document/elements/settings',{{container,settings:{{'{key}':{val_js}}}}});return 'ok'}})()"#
    )
}

fn js_get_preview_box(selector: &str) -> String {
    format!(
        r#"(()=>{{const iframe=document.querySelector('#elementor-preview-iframe');if(!iframe)return JSON.stringify({{error:'no preview iframe'}});const doc=iframe.contentDocument;const el=doc.querySelector('{selector}');if(!el)return JSON.stringify({{error:'not found'}});const b=el.getBoundingClientRect();const s=iframe.contentWindow.getComputedStyle(el);return JSON.stringify({{box:{{x:Math.round(b.x),y:Math.round(b.y),w:Math.round(b.width),h:Math.round(b.height)}},styles:{{'background-color':s.backgroundColor,color:s.color,'font-size':s.fontSize,padding:s.padding,margin:s.margin}},text:el.textContent?.trim()?.substring(0,200)||''}})}})()"#
    )
}

fn js_save_document() -> &'static str {
    r#"(()=>{$e.run('document/save/default');return 'saving'})()"#
}

fn js_wait_for_elementor_ready() -> &'static str {
    r#"new Promise((resolve)=>{const check=()=>{if(window.elementor&&elementor.loaded)resolve('ready');else setTimeout(check,500)};check();setTimeout(()=>resolve('timeout'),30000)})"#
}

#[async_trait]
impl Tool for ElementorEditor {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "elementor_editor",
            description: "Control the Elementor editor via CDP — open editor, select widgets, change settings, read preview, save. Sub-second iteration loop.",
            input_schema: json!({
                "type": "object",
                "required": ["action"],
                "properties": {
                    "action": { "type": "string", "enum": ["open", "select_widget", "set_setting", "get_preview_box", "save"], "description": "Editor action to perform" },
                    "page_id": { "type": "integer", "description": "Required for 'open'" },
                    "element_id": { "type": "string", "description": "8-char Elementor element ID. Required for 'select_widget' and 'set_setting'" },
                    "key": { "type": "string", "description": "Setting key for 'set_setting'" },
                    "value": { "description": "Setting value for 'set_setting'" },
                    "selector": { "type": "string", "description": "CSS selector for 'get_preview_box'" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let action = args["action"].as_str().ok_or_else(|| anyhow::anyhow!("action required"))?;

        match action {
            "open" => {
                let page_id = args["page_id"].as_u64().ok_or_else(|| anyhow::anyhow!("page_id required"))?;
                let url = format!("{}/wp-admin/post.php?post={page_id}&action=elementor", wp.base_url());
                let page = cdp::open_page(&url, 1440, 900).await?;

                let status: String = page.evaluate(js_wait_for_elementor_ready()).await?.into_value().unwrap_or_else(|_| "error".into());
                if status == "timeout" {
                    anyhow::bail!("Elementor editor did not load within 30s. You may need to log in first.");
                }
                Ok(ToolResult::text(format!("Editor opened for page {page_id}")))
            }
            "select_widget" => {
                let eid = args["element_id"].as_str().ok_or_else(|| anyhow::anyhow!("element_id required"))?;
                let page = active_editor_page().await?;
                let result: String = page.evaluate(js_select_widget(eid)).await?.into_value()?;
                if result == "not_found" {
                    anyhow::bail!("Element {eid} not found in editor");
                }
                Ok(ToolResult::text(format!("Selected widget {eid}")))
            }
            "set_setting" => {
                let eid = args["element_id"].as_str().ok_or_else(|| anyhow::anyhow!("element_id required"))?;
                let key = args["key"].as_str().ok_or_else(|| anyhow::anyhow!("key required"))?;
                let val_js = serde_json::to_string(&args["value"])?;
                let page = active_editor_page().await?;
                let result: String = page.evaluate(js_set_setting(eid, key, &val_js)).await?.into_value()?;
                if result == "not_found" {
                    anyhow::bail!("Element {eid} not found");
                }
                Ok(ToolResult::text(format!("Set {key} on {eid}")))
            }
            "get_preview_box" => {
                let selector = args["selector"].as_str().ok_or_else(|| anyhow::anyhow!("selector required"))?;
                let page = active_editor_page().await?;
                let result: String = page.evaluate(js_get_preview_box(selector)).await?.into_value()?;
                let val: Value = serde_json::from_str(&result)?;
                if let Some(err) = val.get("error") {
                    anyhow::bail!("{}", err.as_str().unwrap_or("unknown"));
                }
                Ok(ToolResult::text(serde_json::to_string_pretty(&val)?))
            }
            "save" => {
                let page = active_editor_page().await?;
                let _: String = page.evaluate(js_save_document()).await?.into_value()?;
                Ok(ToolResult::text("Save triggered"))
            }
            _ => anyhow::bail!("Unknown action: {action}"),
        }
    }
}
