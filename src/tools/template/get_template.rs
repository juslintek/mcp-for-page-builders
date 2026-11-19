use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::u64_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct GetTemplate;
#[async_trait]
impl Tool for GetTemplate {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_template",
            description: "Get an Elementor template by ID including its data.",
            input_schema: json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"}}}),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let tpl = wp
            .get(&format!("wp/v2/elementor_library/{id}?context=edit"))
            .await?;
        let metadata = wp
            .get(&format!(
                "mcp-for-page-builders/v1/resources/post_metadata/{id}"
            ))
            .await?;
        let mut tpl = tpl;
        tpl["metadata_revision"] = metadata["revision"].clone();
        tpl["metadata_resource"] = metadata["value"].clone();
        let deletion = wp
            .get(&format!("mcp-for-page-builders/v1/resources/trash/{id}"))
            .await?;
        tpl["deletion_revision"] = deletion["revision"].clone();
        Ok(ToolResult::text(serde_json::to_string_pretty(&tpl)?))
    }
}
