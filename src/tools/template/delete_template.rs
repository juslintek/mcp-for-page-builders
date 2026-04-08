use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::u64_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct DeleteTemplate;
#[async_trait]
impl Tool for DeleteTemplate {
    fn def(&self) -> ToolDef {
        ToolDef { name: "delete_template", description: "Delete an Elementor template.",
            input_schema: json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"},"force":{"type":"boolean","default":false}}}) }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let force = args.get("force").and_then(serde_json::Value::as_bool).unwrap_or(false);
        let path = if force { format!("wp/v2/elementor_library/{id}?force=true") } else { format!("wp/v2/elementor_library/{id}") };
        wp.delete(&path).await?;
        Ok(ToolResult::text(format!("Deleted template {id}")))
    }
}
