use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct DeleteGlobalColor;

#[async_trait]
impl Tool for DeleteGlobalColor {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "delete_global_color",
            description: "Delete a global color by ID.",
            input_schema: json!({
                "type": "object", "required": ["id"],
                "properties": { "id": { "type": "string", "description": "Color ID to delete" } }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = str_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        wp.delete(&format!("elementor/v1/globals/colors/{id}")).await?;
        Ok(ToolResult::text(format!("Deleted global color '{id}'")))
    }
}
