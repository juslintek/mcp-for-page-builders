use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::u64_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct GetPage;

#[async_trait]
impl Tool for GetPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_page",
            description: "Get a WordPress page by ID including its Elementor data.\n\nWorkflow: Use to read current page state. The _elementor_data field contains the element tree as a JSON string.",
            input_schema: json!({
                "type": "object",
                "required": ["id"],
                "properties": { "id": { "type": "integer", "description": "Page ID" } }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let page = wp.get(&format!("wp/v2/pages/{id}?context=edit")).await?;
        let metadata = wp
            .get(&format!(
                "mcp-for-page-builders/v1/resources/post_metadata/{id}"
            ))
            .await?;
        let mut page = page;
        page["metadata_revision"] = metadata["revision"].clone();
        page["metadata_resource"] = metadata["value"].clone();
        let deletion = wp
            .get(&format!("mcp-for-page-builders/v1/resources/trash/{id}"))
            .await?;
        page["deletion_revision"] = deletion["revision"].clone();
        Ok(ToolResult::text(serde_json::to_string_pretty(&page)?))
    }
}
