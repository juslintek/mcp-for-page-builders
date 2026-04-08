use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct RemoveElement;

#[async_trait]
impl Tool for RemoveElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "remove_element",
            description: "Remove an element by ID from a page's element tree.",
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
        match tree.remove(&eid) {
            Some(_) => {
                svc.save_tree(page_id, &tree).await?;
                Ok(ToolResult::text(format!("Removed element {eid} from page {page_id}")))
            }
            None => Ok(ToolResult::error(format!("Element {eid} not found on page {page_id}"))),
        }
    }
}
