use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct GetPageBySlug;

#[async_trait]
impl Tool for GetPageBySlug {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_page_by_slug",
            description: "Look up a page ID from its URL slug.",
            input_schema: json!({
                "type": "object",
                "required": ["slug"],
                "properties": { "slug": { "type": "string", "description": "URL slug (without slashes)" } }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let slug = str_arg(&args, "slug").ok_or_else(|| anyhow::anyhow!("slug required"))?;
        let result = wp.get(&format!("wp/v2/pages?slug={slug}&context=edit")).await?;
        let pages = result.as_array().ok_or_else(|| anyhow::anyhow!("Unexpected response"))?;
        if pages.is_empty() {
            return Ok(ToolResult::error(format!("No page found with slug '{slug}'")));
        }
        let id = pages[0]["id"].as_u64().unwrap_or(0);
        let title = pages[0]["title"]["rendered"].as_str().unwrap_or("");
        Ok(ToolResult::text(format!("Page ID {id}: {title}")))
    }
}
