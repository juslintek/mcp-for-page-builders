use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg, usize_arg};
use crate::elementor::{Element, ElementorService};
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct AddElement;

#[async_trait]
impl Tool for AddElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "add_element",
            description: "Insert a new element (widget/container) into a page at a specific position. Provide the element as JSON.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "element"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "parent_id": { "type": "string", "description": "Parent element ID. Omit for root level." },
                    "position": { "type": "integer", "description": "Index to insert at (0-based). Defaults to end." },
                    "element": { "type": "object", "description": "Full element JSON with elType, settings, etc. ID auto-generated if missing." }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let parent_id = str_arg(&args, "parent_id");
        let position = usize_arg(&args, "position").unwrap_or(usize::MAX);

        let el_json = args.get("element").ok_or_else(|| anyhow::anyhow!("element required"))?;
        let mut new_el: Element = serde_json::from_value(el_json.clone())?;
        if new_el.id.is_empty() {
            new_el.id = crate::elementor::generate_id();
        }
        let new_id = new_el.id.clone();

        let svc = ElementorService::new(wp);
        let mut tree = svc.get_tree(page_id).await?;
        if !tree.insert(parent_id.as_deref(), position, new_el) {
            return Ok(ToolResult::error(format!("Parent {} not found", parent_id.unwrap_or_default())));
        }
        svc.save_tree(page_id, &tree).await?;

        Ok(ToolResult::text(format!("Added element {new_id} to page {page_id}")))
    }
}
