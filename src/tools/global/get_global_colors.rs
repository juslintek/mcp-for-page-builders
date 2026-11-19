use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct GetGlobalColors;

#[async_trait]
impl Tool for GetGlobalColors {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_global_colors",
            description: "Get all Elementor global colors (design tokens).",
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let globals = wp.get("elementor/v1/globals").await?;
        let colors = globals.get("colors").cloned().unwrap_or_else(|| json!({}));
        let settings = super::resource::kit_settings(wp).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(
            &json!({"revision":settings["revision"], "colors":colors}),
        )?))
    }
}
