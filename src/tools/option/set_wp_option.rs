use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct SetWpOption;
#[async_trait]
impl Tool for SetWpOption {
    fn def(&self) -> ToolDef {
        ToolDef { name: "set_wp_option", description: "Write a WordPress option value. Uses the elementor-mcp/v1/option/ REST endpoint (requires the companion mu-plugin).",
            input_schema: json!({"type":"object","required":["name","value"],"properties":{"name":{"type":"string"},"value":{"description":"Option value (string, number, object, or array)"}}}) }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let name = str_arg(&args, "name").ok_or_else(|| anyhow::anyhow!("name required"))?;
        let value = args.get("value").ok_or_else(|| anyhow::anyhow!("value required"))?;
        wp.post(&format!("elementor-mcp/v1/option/{name}"), value).await?;
        Ok(ToolResult::text(format!("Option '{name}' updated")))
    }
}
