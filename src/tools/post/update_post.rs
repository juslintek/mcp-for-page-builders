use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::{Tool, elementor_plan_tools::ApplyElementorPlan};
use crate::wp::WpClient;

pub struct UpdatePost;
#[async_trait]
impl Tool for UpdatePost {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "update_post",
            description: "Update a WordPress post.",
            input_schema: json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"},"title":{"type":"string"},"content":{"type":"string"},"status":{"type":"string","enum":["publish","draft","private"]},"expected_metadata_revision":{"type":"string"},"operation_id":{"type":"string"}}}),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let mut body = json!({});
        if let Some(v) = str_arg(&args, "title") {
            body["title"] = json!(v);
        }
        if let Some(v) = str_arg(&args, "content") {
            body["content"] = json!(v);
        }
        if let Some(v) = str_arg(&args, "status") {
            body["status"] = json!(v);
        }
        if body == json!({}) {
            anyhow::bail!("Nothing to update");
        }
        let state = wp
            .get(&format!(
                "mcp-for-page-builders/v1/resources/post_metadata/{id}"
            ))
            .await?;
        let value = state
            .get("value")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                anyhow::anyhow!("Invalid post metadata inspection response; update blocked")
            })?;
        if !state
            .get("revision")
            .and_then(Value::as_str)
            .is_some_and(|revision| !revision.is_empty())
            || value
                .get("protected_elementor")
                .and_then(Value::as_bool)
                .is_none()
            || !value.get("title").and_then(Value::as_str).is_some()
            || !value.get("status").and_then(Value::as_str).is_some()
        {
            anyhow::bail!("Incomplete post metadata inspection response; update blocked");
        }
        if value.get("post_type").and_then(Value::as_str) != Some("post") {
            anyhow::bail!("update_post only accepts WordPress post resources; update blocked");
        }
        if value["protected_elementor"] == true {
            if body.get("content").is_some() {
                anyhow::bail!("Content updates are unsupported for Elementor-backed posts");
            }
            let expected = str_arg(&args, "expected_metadata_revision").ok_or_else(|| {
                anyhow::anyhow!("expected_metadata_revision required; read the post first")
            })?;
            let tree = ElementorService::new(wp).get_tree(id).await?;
            let document_revision = tree
                .revision
                .ok_or_else(|| anyhow::anyhow!("Document revision unavailable; update blocked"))?;
            let mut after = json!({"title":value.get("title"),"status":value.get("status")});
            for field in ["title", "status"] {
                if let Some(patch) = body.get(field) {
                    after[field] = patch.clone();
                }
            }
            let operation_id = str_arg(&args, "operation_id")
                .unwrap_or_else(|| format!("mcp-{}", crate::elementor::generate_id()));
            return ApplyElementorPlan.run(json!({"plan":{"site_url":wp.base_url(),"page_id":id,"expected_revision":document_revision,"operation_id":operation_id,"resource_patches":[{"kind":"post_metadata","id":id.to_string(),"expected_revision":expected,"after":after}]}}), wp).await;
        }
        wp.post(&format!("wp/v2/posts/{id}"), &body).await?;
        Ok(ToolResult::text(format!("Updated post {id}")))
    }
}
