use std::fmt::Write;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct ListPosts;
#[async_trait]
impl Tool for ListPosts {
    fn def(&self) -> ToolDef {
        ToolDef { name: "list_posts", description: "List WordPress posts.",
            input_schema: json!({"type":"object","properties":{"per_page":{"type":"integer","default":20},"page":{"type":"integer","default":1},"status":{"type":"string","default":"any"},"search":{"type":"string"}}}) }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let per_page = args.get("per_page").and_then(serde_json::Value::as_u64).unwrap_or(20);
        let page = args.get("page").and_then(serde_json::Value::as_u64).unwrap_or(1);
        let status = str_arg(&args, "status").unwrap_or_else(|| "any".into());
        let mut url = format!("wp/v2/posts?per_page={per_page}&page={page}&status={status}");
        if let Some(s) = str_arg(&args, "search") { write!(url, "&search={s}").unwrap(); }
        let result = wp.get(&url).await?;
        let posts = result.as_array().ok_or_else(|| anyhow::anyhow!("Unexpected response"))?;
        let mut lines = vec![format!("Found {} posts:", posts.len())];
        for p in posts {
            let id = p["id"].as_u64().unwrap_or(0);
            let title = p["title"]["rendered"].as_str().unwrap_or("(no title)");
            let status = p["status"].as_str().unwrap_or("");
            lines.push(format!("  [{id}] {title} ({status})"));
        }
        Ok(ToolResult::text(lines.join("\n")))
    }
}
