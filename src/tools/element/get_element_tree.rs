use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::u64_arg;
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct GetElementTree;

#[async_trait]
impl Tool for GetElementTree {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_element_tree",
            description: "Get a flattened view of a page's element tree showing paths, types, IDs, and the expected_revision required by every mutation tool.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id"],
                "properties": { "page_id": { "type": "integer" } }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id =
            u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;

        let tree = ElementorService::new(wp).get_tree(page_id).await?;
        let flat = tree.flatten();

        let lines: Vec<String> = flat
            .iter()
            .map(|(path, label)| format!("{path}  {label}"))
            .collect();
        Ok(ToolResult::text(serde_json::to_string_pretty(&json!({
            "revision": tree.revision,
            "elements": lines,
        }))?))
    }
}
