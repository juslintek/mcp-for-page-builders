use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct GetKitSchema;

#[async_trait]
impl Tool for GetKitSchema {
    fn def(&self) -> ToolDef {
        ToolDef { name: "get_kit_schema", description: "Get the Elementor kit schema — all available kit settings, their types, and defaults. Useful for discovering what can be configured.", input_schema: json!({ "type": "object", "properties": {} }) }
    }
    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let schema = wp.get("angie/v1/elementor-kit/schema").await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&schema)?))
    }
}
