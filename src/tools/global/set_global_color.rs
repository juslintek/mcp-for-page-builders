use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct SetGlobalColor;

#[async_trait]
impl Tool for SetGlobalColor {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "set_global_color",
            description: "Create or update a global color. Use the color ID to update existing.",
            input_schema: json!({
                "type": "object",
                "required": ["id", "title", "color"],
                "properties": {
                    "id": { "type": "string", "description": "Color ID (e.g. 'primary', 'secondary', or custom ID)" },
                    "title": { "type": "string", "description": "Display name" },
                    "color": { "type": "string", "description": "Hex color value (e.g. '#0C91BA')" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = str_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let title = str_arg(&args, "title").ok_or_else(|| anyhow::anyhow!("title required"))?;
        let color = str_arg(&args, "color").ok_or_else(|| anyhow::anyhow!("color required"))?;

        let body = json!({ "id": id, "title": title, "value": color });
        wp.post(&format!("elementor/v1/globals/colors/{id}"), &body).await?;
        Ok(ToolResult::text(format!("Set global color '{id}': {title} = {color}")))
    }
}
