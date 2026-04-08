use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::u64_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct GetTemplate;
#[async_trait]
impl Tool for GetTemplate {
    fn def(&self) -> ToolDef {
        ToolDef { name: "get_template", description: "Get an Elementor template by ID including its data.",
            input_schema: json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"}}}) }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let tpl = wp.get(&format!("wp/v2/elementor_library/{id}?context=edit")).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&tpl)?))
    }
}
