use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct DeleteGlobalTypography;

#[async_trait]
impl Tool for DeleteGlobalTypography {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "delete_global_typography",
            description: "Delete a global typography preset by ID.",
            input_schema: json!({
                "type": "object", "required": ["id"],
                "properties": { "id": { "type": "string" } }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = str_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        wp.delete(&format!("elementor/v1/globals/typography/{id}")).await?;
        Ok(ToolResult::text(format!("Deleted global typography '{id}'")))
    }
}
