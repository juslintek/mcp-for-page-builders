use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct GetExperiments;

#[async_trait]
impl Tool for GetExperiments {
    fn def(&self) -> ToolDef {
        ToolDef { name: "get_experiments", description: "Get all Elementor experiments (feature flags) and their current state.", input_schema: json!({ "type": "object", "properties": {} }) }
    }
    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let settings = wp.get("elementor/v1/settings").await?;
        let experiments = settings.get("experiments").cloned().unwrap_or_else(|| json!({}));
        Ok(ToolResult::text(serde_json::to_string_pretty(&experiments)?))
    }
}
