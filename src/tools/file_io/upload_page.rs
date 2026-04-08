use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct UploadPage;

#[async_trait]
impl Tool for UploadPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "upload_page",
            description: "Update a page's Elementor data from a local JSON file. Clears CSS cache automatically.",
            input_schema: json!({"type":"object","required":["id","path"],"properties":{"id":{"type":"integer"},"path":{"type":"string"}}}),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let path = str_arg(&args, "path").ok_or_else(|| anyhow::anyhow!("path required"))?;

        let content = tokio::fs::read_to_string(&path).await
            .map_err(|e| anyhow::anyhow!("Failed to read {path}: {e}"))?;
        let parsed: Value = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("File is not valid JSON: {e}"))?;
        let compact = serde_json::to_string(&parsed)?;

        let body = json!({"meta": {"_elementor_data": compact}});
        wp.post(&format!("wp/v2/pages/{id}"), &body).await?;
        ElementorService::new(wp).clear_cache().await?;

        Ok(ToolResult::text(format!("Updated page {id} from {path} and cleared CSS cache.")))
    }
}
