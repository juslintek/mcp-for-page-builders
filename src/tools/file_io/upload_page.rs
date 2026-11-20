use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct UploadPage;

#[async_trait]
impl Tool for UploadPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "upload_page",
            description: "Update a page's Elementor data from a local JSON file. Clears CSS cache automatically.",
            input_schema: json!({"type":"object","required":["id","path","expected_revision"],"properties":{"id":{"type":"integer"},"path":{"type":"string"},"expected_revision":{"type":"string","description":"Revision returned by the page tree read"}}}),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let path = str_arg(&args, "path").ok_or_else(|| anyhow::anyhow!("path required"))?;
        let expected_revision = str_arg(&args, "expected_revision").ok_or_else(|| {
            anyhow::anyhow!("expected_revision required; read the page tree first")
        })?;

        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read {path}: {e}"))?;
        let parsed: Value = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("File is not valid JSON: {e}"))?;
        let elements: Vec<crate::types::Element> = serde_json::from_value(parsed)
            .map_err(|e| anyhow::anyhow!("File is not a valid typed Elementor document: {e}"))?;
        let tree = ElementorService::new(wp).get_tree(id).await?;
        ElementorService::require_revision(&tree, &expected_revision)?;
        crate::elementor::set_page_elements_checked(
            wp,
            id,
            &elements,
            &expected_revision,
            &format!("mcp-{}", crate::elementor::generate_id()),
        )
        .await?;
        let cache =
            crate::cache::clear_related_caches(wp, crate::cache::CacheScope::Post(id)).await;

        Ok(ToolResult::text(format!(
            "Updated page {id} from {path}{}",
            cache.summary_line()
        )))
    }
}
