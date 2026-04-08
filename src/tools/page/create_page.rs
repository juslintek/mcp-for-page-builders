use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct CreatePage;

#[async_trait]
impl Tool for CreatePage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "create_page",
            description: "Create a new WordPress page with Elementor data. Returns the new page ID.",
            input_schema: json!({
                "type": "object",
                "required": ["title", "elementor_data"],
                "properties": {
                    "title": { "type": "string", "description": "Page title" },
                    "elementor_data": { "type": "string", "description": "Elementor JSON array string" },
                    "status": { "type": "string", "enum": ["publish","draft","private"], "default": "draft" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let title = str_arg(&args, "title").unwrap_or_default();
        let data = str_arg(&args, "elementor_data").unwrap_or_default();
        let status = str_arg(&args, "status").unwrap_or_else(|| "draft".into());

        serde_json::from_str::<Value>(&data)
            .map_err(|e| anyhow::anyhow!("elementor_data is not valid JSON: {e}"))?;

        let body = json!({
            "title": title,
            "status": status,
            "meta": { "_elementor_data": data, "_elementor_edit_mode": "builder" }
        });

        let result = wp.post("wp/v2/pages", &body).await?;
        let id = result["id"].as_u64().unwrap_or(0);
        ElementorService::new(wp).clear_cache().await?;

        Ok(ToolResult::text(format!("Created page ID {id}: {}", result["link"].as_str().unwrap_or(""))))
    }
}
