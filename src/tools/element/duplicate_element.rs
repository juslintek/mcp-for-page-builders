use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct DuplicateElement;

#[async_trait]
impl Tool for DuplicateElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "duplicate_element",
            description: "Clone an element (and all children) with new IDs, inserted right after the original.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "element_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "element_id": { "type": "string" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid = str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;

        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;

        let mut clone = tree.find(&eid)
            .ok_or_else(|| anyhow::anyhow!("Element {eid} not found"))?;
        crate::elementor::regenerate_ids(&mut clone);
        let clone_id = clone.id.clone();

        if !tree.insert_after(&eid, clone) {
            return Ok(ToolResult::error(format!("Could not insert clone after {eid}")));
        }

        svc.save_tree(page_id, &tree).await?;
        Ok(ToolResult::text(format!("Duplicated {eid} → {clone_id} on page {page_id}")))
    }
}
