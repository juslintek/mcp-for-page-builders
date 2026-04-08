use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct CreatePost;
#[async_trait]
impl Tool for CreatePost {
    fn def(&self) -> ToolDef {
        ToolDef { name: "create_post", description: "Create a new WordPress post.",
            input_schema: json!({"type":"object","required":["title"],"properties":{"title":{"type":"string"},"content":{"type":"string"},"status":{"type":"string","enum":["publish","draft","private"],"default":"draft"},"categories":{"type":"array","items":{"type":"integer"}},"tags":{"type":"array","items":{"type":"integer"}}}}) }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let mut body = json!({"title": str_arg(&args, "title").unwrap_or_default()});
        if let Some(v) = str_arg(&args, "content") { body["content"] = json!(v); }
        if let Some(v) = str_arg(&args, "status") { body["status"] = json!(v); }
        if let Some(v) = args.get("categories") { body["categories"] = v.clone(); }
        if let Some(v) = args.get("tags") { body["tags"] = v.clone(); }
        let result = wp.post("wp/v2/posts", &body).await?;
        let id = result["id"].as_u64().unwrap_or(0);
        Ok(ToolResult::text(format!("Created post {id}: {}", result["link"].as_str().unwrap_or(""))))
    }
}
