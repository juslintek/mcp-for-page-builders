use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg};
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct UpdatePost;
#[async_trait]
impl Tool for UpdatePost {
    fn def(&self) -> ToolDef {
        ToolDef { name: "update_post", description: "Update a WordPress post.",
            input_schema: json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"},"title":{"type":"string"},"content":{"type":"string"},"status":{"type":"string","enum":["publish","draft","private"]}}}) }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let mut body = json!({});
        if let Some(v) = str_arg(&args, "title") { body["title"] = json!(v); }
        if let Some(v) = str_arg(&args, "content") { body["content"] = json!(v); }
        if let Some(v) = str_arg(&args, "status") { body["status"] = json!(v); }
        wp.post(&format!("wp/v2/posts/{id}"), &body).await?;
        Ok(ToolResult::text(format!("Updated post {id}")))
    }
}
