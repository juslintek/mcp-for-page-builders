use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct GetElement;

#[async_trait]
impl Tool for GetElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_element",
            description: "Get a single Elementor element by ID from a page's element tree.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id", "element_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "element_id": { "type": "string", "description": "8-char hex element ID" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let eid = str_arg(&args, "element_id").ok_or_else(|| anyhow::anyhow!("element_id required"))?;

        let tree = ElementorService::new(wp).get_tree(page_id).await?;
        match tree.find(&eid) {
            Some(el) => Ok(ToolResult::text(serde_json::to_string_pretty(&el)?)),
            None => Ok(ToolResult::error(format!("Element {eid} not found on page {page_id}"))),
        }
    }
}
