use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct DeleteGlobalColor;

#[async_trait]
impl Tool for DeleteGlobalColor {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "delete_global_color",
            description: "Delete a global color by ID.",
            input_schema: json!({
                "type": "object", "required": ["id", "expected_revision"],
                "properties": { "id": { "type": "string", "description": "Color ID to delete" }, "expected_revision": { "type": "string", "description": "Revision returned by get_global_colors" } }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = str_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let revision = str_arg(&args, "expected_revision").ok_or_else(|| {
            anyhow::anyhow!("expected_revision required; read global colors first")
        })?;
        let mut settings = super::resource::kit_settings(wp).await?;
        let collection =
            super::resource::collection(&settings, "system_colors", "custom_colors", &id);
        super::resource::remove(&mut settings, &collection, &id)?;
        super::resource::save_kit_settings(wp, settings, &revision).await?;
        Ok(ToolResult::text(format!("Deleted global color '{id}'")))
    }
}
