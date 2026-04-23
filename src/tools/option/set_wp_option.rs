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
        ToolDef { name: "set_wp_option", description: "Write a WordPress option value. Uses the mcp-for-page-builders/v1/option/ REST endpoint (requires the companion mu-plugin).",
            input_schema: json!({"type":"object","required":["name","value"],"properties":{"name":{"type":"string"},"value":{"description":"Option value (string, number, object, or array)"}}}) }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let name = str_arg(&args, "name").ok_or_else(|| anyhow::anyhow!("name required"))?;
        let value = args.get("value").ok_or_else(|| anyhow::anyhow!("value required"))?;
        let jid = wp.session.as_ref().map(|s| s.record("set_wp_option", wp.base_url(), &format!("option:{name}")));
        wp.post(&format!("mcp-for-page-builders/v1/option/{name}"), value).await?;
        if let (Some(s), Some(id)) = (&wp.session, jid) { s.complete(&id); }
        Ok(ToolResult::text(format!("Option '{name}' updated")))
    }
}
