use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg, usize_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct MoveElement;

#[async_trait]
impl Tool for MoveElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "move_element",
            description: "Move an element to a different parent and/or position within the page.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "element_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "element_id": { "type": "string", "description": "Element to move" },
                    "target_parent_id": { "type": "string", "description": "New parent ID. Omit for root level." },
                    "position": { "type": "integer", "description": "Index in new parent. Defaults to end." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid = str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;
        let target = str_arg(&args, "target_parent_id");
        let position = usize_arg(&args, "position").unwrap_or(usize::MAX);

        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;

        let el = tree.remove(&eid)
            .ok_or_else(|| anyhow::anyhow!("Element {eid} not found"))?;

        if !tree.insert(target.as_deref(), position, el) {
            return Ok(ToolResult::error(format!("Target parent {} not found", target.unwrap_or_default())));
        }

        svc.save_tree(page_id, &tree).await?;
        Ok(ToolResult::text(format!("Moved element {eid} on page {page_id}")))
    }
}
