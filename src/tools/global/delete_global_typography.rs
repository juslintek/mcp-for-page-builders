use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct DeleteGlobalTypography;

#[async_trait]
impl Tool for DeleteGlobalTypography {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "delete_global_typography",
            description: "Delete a global typography preset by ID.",
            input_schema: json!({
                "type": "object", "required": ["id", "expected_revision"],
                "properties": { "id": { "type": "string" }, "expected_revision": { "type": "string", "description": "Revision returned by get_global_typography" } }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = str_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let revision = str_arg(&args, "expected_revision").ok_or_else(|| {
            anyhow::anyhow!("expected_revision required; read global typography first")
        })?;
        let mut settings = super::resource::kit_settings(wp).await?;
        let collection =
            super::resource::collection(&settings, "system_typography", "custom_typography", &id);
        super::resource::remove(&mut settings, &collection, &id)?;
        super::resource::save_kit_settings(wp, settings, &revision).await?;
        Ok(ToolResult::text(format!(
            "Deleted global typography '{id}'"
        )))
    }
}
