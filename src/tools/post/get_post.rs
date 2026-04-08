use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::u64_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct GetPost;
#[async_trait]
impl Tool for GetPost {
    fn def(&self) -> ToolDef {
        ToolDef { name: "get_post", description: "Get a WordPress post by ID.",
            input_schema: json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"}}}) }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let post = wp.get(&format!("wp/v2/posts/{id}?context=edit")).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&post)?))
    }
}
