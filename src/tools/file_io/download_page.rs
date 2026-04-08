use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg};
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct DownloadPage;

#[async_trait]
impl Tool for DownloadPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "download_page",
            description: "Download a page's Elementor data to a local JSON file.",
            input_schema: json!({"type":"object","required":["id","path"],"properties":{"id":{"type":"integer","description":"Page ID"},"path":{"type":"string","description":"Local file path to save to"}}}),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let path = str_arg(&args, "path").ok_or_else(|| anyhow::anyhow!("path required"))?;

        let page = wp.get(&format!("wp/v2/pages/{id}?context=edit")).await?;
        let data = page.get("meta").and_then(|m| m.get("_elementor_data"))
            .ok_or_else(|| anyhow::anyhow!("No _elementor_data found on page {id}"))?;

        let content = match data.as_str() {
            Some(s) => { let parsed: Value = serde_json::from_str(s)?; serde_json::to_string_pretty(&parsed)? }
            None => serde_json::to_string_pretty(data)?,
        };

        tokio::fs::write(&path, &content).await?;
        Ok(ToolResult::text(format!("Saved Elementor data for page {id} to {path} ({} bytes)", content.len())))
    }
}
