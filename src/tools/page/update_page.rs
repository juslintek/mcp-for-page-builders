use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct UpdatePage;

#[async_trait]
impl Tool for UpdatePage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "update_page",
            description: "Update a page's title, status, and/or Elementor data. Clears CSS cache automatically.",
            input_schema: json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "integer" },
                    "title": { "type": "string" },
                    "status": { "type": "string", "enum": ["publish","draft","private"] },
                    "elementor_data": { "type": "string", "description": "Full Elementor JSON array string" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let mut body = json!({});

        if let Some(t) = str_arg(&args, "title") { body["title"] = json!(t); }
        if let Some(s) = str_arg(&args, "status") { body["status"] = json!(s); }
        if let Some(data) = str_arg(&args, "elementor_data") {
            serde_json::from_str::<Value>(&data)
                .map_err(|e| anyhow::anyhow!("elementor_data is not valid JSON: {e}"))?;
            body["meta"] = json!({ "_elementor_data": data });
        }

        let jid = wp.session.as_ref().map(|s| s.record("update_page", wp.base_url(), &format!("page:{id}")));
        wp.post(&format!("wp/v2/pages/{id}"), &body).await?;
        ElementorService::new(wp).clear_cache().await?;
        if let (Some(s), Some(jid)) = (&wp.session, jid) { s.complete(&jid); }

        Ok(ToolResult::text(format!("Updated page {id} and cleared Elementor CSS cache.")))
    }
}
