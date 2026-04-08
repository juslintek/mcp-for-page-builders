use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct GetKitDefaults;

#[async_trait]
impl Tool for GetKitDefaults {
    fn def(&self) -> ToolDef {
        ToolDef { name: "get_kit_defaults", description: "Get Elementor kit element defaults — the default settings applied to all widgets of each type.", input_schema: json!({ "type": "object", "properties": {} }) }
    }
    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let defaults = wp.get("elementor/v1/kit-elements-defaults").await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&defaults)?))
    }
}
