use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct FindElements;

#[async_trait]
impl Tool for FindElements {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "find_elements",
            description: "Search for elements by widget type and/or setting key/value.",
            input_schema: json!({
                "type": "object",
                "required": ["page_id"],
                "properties": {
                    "page_id": { "type": "integer" },
                    "widget_type": { "type": "string", "description": "Filter by widget type (e.g. 'heading', 'text-editor')" },
                    "setting_key": { "type": "string", "description": "Filter by setting key existence or value" },
                    "setting_value": { "type": "string", "description": "Required value for setting_key" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let page_id = u64_arg(&args, "page_id").ok_or_else(|| anyhow::anyhow!("page_id required"))?;
        let wt = str_arg(&args, "widget_type");
        let sk = str_arg(&args, "setting_key");
        let sv = str_arg(&args, "setting_value");

        let tree = ElementorService::new(wp).get_tree(page_id).await?;
        let results = tree.search(wt.as_deref(), sk.as_deref(), sv.as_deref());

        if results.is_empty() {
            return Ok(ToolResult::text("No matching elements found."));
        }

        let mut lines = vec![format!("Found {} elements:", results.len())];
        for el in &results {
            let wt = el.widget_type.as_deref().unwrap_or("-");
            lines.push(format!("  {} ({}) id={}", el.el_type, wt, el.id));
        }
        Ok(ToolResult::text(lines.join("\n")))
    }
}
