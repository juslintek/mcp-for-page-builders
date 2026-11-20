use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::{str_arg, u64_arg};
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct DeletePage;

#[async_trait]
impl Tool for DeletePage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "delete_page",
            description: "Move a WordPress page to trash with a checked revision and restore journal.",
            input_schema: json!({
                "type": "object",
                "required": ["id", "expected_revision"],
                "properties": {
                    "id": { "type": "integer" },
                    "expected_revision": { "type": "string", "description": "deletion_revision returned by get_page" },
                    "operation_id": { "type": "string" },
                    "force": { "type": "boolean", "description": "Bypass trash", "default": false }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let force = args
            .get("force")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if force {
            anyhow::bail!(
                "force=true permanent deletion is unavailable because faithful restoration cannot be guaranteed; no changes were made"
            );
        }
        let expected = str_arg(&args, "expected_revision").ok_or_else(|| {
            anyhow::anyhow!(
                "expected_revision is required; read deletion_revision from get_page first"
            )
        })?;
        let operation_id = str_arg(&args, "operation_id")
            .unwrap_or_else(|| format!("delete-{}", crate::elementor::generate_id()));
        let receipt = wp.request("POST", "mcp-for-page-builders/v1/resources/apply", Some(&json!({"operation_id":operation_id,"resource_patches":[{"kind":"trash","id":id.to_string(),"expected_revision":expected,"after":{}}]})), None).await?;
        let mut resp = json!({"trashed":true,"id":id,"operation_id":operation_id,"restore_operation_id":operation_id,"receipt":receipt});
        crate::cache::clear_and_attach(wp, crate::cache::CacheScope::Post(id), &mut resp).await;
        Ok(ToolResult::text(serde_json::to_string(&resp)?))
    }
}
