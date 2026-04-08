use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct UpdateElement;

#[async_trait]
impl Tool for UpdateElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "update_element",
            description: "Update an element's settings by ID. Merges provided settings with existing ones (partial update).",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "element_id", "settings"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "element_id": { "type": "string" },
                    "settings": { "type": "object", "description": "Settings to merge into the element" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid = str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;
        let new_settings = args.get("settings").ok_or_else(|| anyhow::anyhow!("settings required"))?.clone();

        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;
        let found = tree.mutate(&eid, |el| crate::elementor::merge_settings(&mut el.settings, &new_settings));

        if !found {
            return Ok(ToolResult::error(format!("Element {eid} not found on page {page_id}")));
        }

        svc.save_tree(page_id, &tree).await?;
        Ok(ToolResult::text(format!("Updated settings for element {eid} on page {page_id}")))
    }
}
