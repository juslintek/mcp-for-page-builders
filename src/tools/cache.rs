use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

pub struct ClearCache;

#[async_trait]
impl Tool for ClearCache {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "clear_cache",
            description: "Clear Elementor's CSS cache and regenerate styles. Call after programmatic page updates.",
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        wp.clear_elementor_cache().await?;
        Ok(ToolResult::text("Elementor CSS cache cleared."))
    }
}
