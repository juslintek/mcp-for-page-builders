use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct GetGlobalTypography;

#[async_trait]
impl Tool for GetGlobalTypography {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_global_typography",
            description: "Get all Elementor global typography presets.",
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let globals = wp.get("elementor/v1/globals").await?;
        let typo = globals.get("typography").cloned().unwrap_or_else(|| json!({}));
        Ok(ToolResult::text(serde_json::to_string_pretty(&typo)?))
    }
}
