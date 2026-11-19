use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::u64_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

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
        let metadata = wp
            .get(&format!(
                "mcp-for-page-builders/v1/resources/post_metadata/{id}"
            ))
            .await?;
        let deletion = wp
            .get(&format!("mcp-for-page-builders/v1/resources/trash/{id}"))
            .await?;
        let mut post = post;
        post["metadata_revision"] = metadata["revision"].clone();
        post["protected_elementor"] = metadata["value"]["protected_elementor"].clone();
        post["deletion_revision"] = deletion["revision"].clone();
        Ok(ToolResult::text(serde_json::to_string_pretty(&post)?))
    }
}
