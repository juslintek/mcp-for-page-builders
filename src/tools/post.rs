use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

fn str_arg(args: &Value, key: &str) -> Option<String> { args.get(key)?.as_str().map(|s| s.to_string()) }
fn u64_arg(args: &Value, key: &str) -> Option<u64> { args.get(key)?.as_u64() }

pub struct CreatePost;
#[async_trait]
impl Tool for CreatePost {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "create_post",
            description: "Create a new WordPress post.",
            input_schema: json!({
                "type": "object", "required": ["title"],
                "properties": {
                    "title": {"type": "string"},
                    "content": {"type": "string", "description": "HTML content"},
                    "status": {"type": "string", "enum": ["publish","draft","private"], "default": "draft"},
                    "categories": {"type": "array", "items": {"type": "integer"}},
                    "tags": {"type": "array", "items": {"type": "integer"}}
                }
            }),
        }
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

pub struct GetPost;
#[async_trait]
impl Tool for GetPost {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_post",
            description: "Get a WordPress post by ID.",
            input_schema: json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"}}}),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let post = wp.get(&format!("wp/v2/posts/{id}?context=edit")).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&post)?))
    }
}

pub struct ListPosts;
#[async_trait]
impl Tool for ListPosts {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_posts",
            description: "List WordPress posts.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "per_page": {"type": "integer", "default": 20},
                    "page": {"type": "integer", "default": 1},
                    "status": {"type": "string", "default": "any"},
                    "search": {"type": "string"}
                }
            }),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let per_page = args.get("per_page").and_then(|v| v.as_u64()).unwrap_or(20);
        let page = args.get("page").and_then(|v| v.as_u64()).unwrap_or(1);
        let status = str_arg(&args, "status").unwrap_or_else(|| "any".into());
        let mut url = format!("wp/v2/posts?per_page={per_page}&page={page}&status={status}");
        if let Some(s) = str_arg(&args, "search") { url.push_str(&format!("&search={s}")); }
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

pub struct UpdatePost;
#[async_trait]
impl Tool for UpdatePost {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "update_post",
            description: "Update a WordPress post.",
            input_schema: json!({
                "type": "object", "required": ["id"],
                "properties": {
                    "id": {"type": "integer"},
                    "title": {"type": "string"},
                    "content": {"type": "string"},
                    "status": {"type": "string", "enum": ["publish","draft","private"]}
                }
            }),
        }
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

pub struct DeletePost;
#[async_trait]
impl Tool for DeletePost {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "delete_post",
            description: "Delete a WordPress post.",
            input_schema: json!({
                "type": "object", "required": ["id"],
                "properties": {
                    "id": {"type": "integer"},
                    "force": {"type": "boolean", "default": false}
                }
            }),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
        let path = if force { format!("wp/v2/posts/{id}?force=true") } else { format!("wp/v2/posts/{id}") };
        wp.delete(&path).await?;
        Ok(ToolResult::text(format!("Deleted post {id}")))
    }
}
