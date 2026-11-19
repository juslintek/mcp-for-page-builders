use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::ToolDef;
use crate::tools::{Tool, elementor_plan_tools::ApplyElementorPlan};
use crate::wp::WpClient;

pub struct UpdateTemplate;
#[async_trait]
impl Tool for UpdateTemplate {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "update_template",
            description: "Update an existing Elementor template title and/or data through checked revisions. Conditions updates are not available in this checked-write phase.",
            input_schema: json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"},"title":{"type":"string"},"elementor_data":{"type":"string"},"expected_revision":{"type":"string","description":"Revision returned by get_element_tree; required with elementor_data"},"expected_metadata_revision":{"type":"string","description":"metadata_revision returned by get_template; required with title"},"operation_id":{"type":"string","description":"Stable operation ID; generated when omitted"},"conditions":{"type":"array","items":{"type":"string"}}}}),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<crate::mcp::ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let title = str_arg(&args, "title");
        let data = str_arg(&args, "elementor_data");
        if args.get("conditions").is_some() {
            anyhow::bail!(
                "conditions updates are unsupported in this checked-write phase; no changes were made"
            );
        }
        if title.is_none() && data.is_none() {
            anyhow::bail!("Nothing to update");
        }
        let operation_id = str_arg(&args, "operation_id")
            .unwrap_or_else(|| format!("mcp-{}", crate::elementor::generate_id()));
        let current = ElementorService::new(wp).get_tree(id).await?;
        let expected_revision = if data.is_some() {
            str_arg(&args, "expected_revision")
                .ok_or_else(|| {
                    anyhow::anyhow!("expected_revision required; read the template tree first")
                })?
                .to_owned()
        } else {
            current.revision.clone().ok_or_else(|| {
                anyhow::anyhow!("Template revision is unavailable; read the template tree again")
            })?
        };
        let document_patch = if let Some(data) = data {
            let parsed: Value = serde_json::from_str(&data)
                .map_err(|e| anyhow::anyhow!("Invalid elementor_data JSON: {e}"))?;
            let elements: Vec<crate::types::Element> = serde_json::from_value(parsed)
                .map_err(|e| anyhow::anyhow!("Invalid typed Elementor document: {e}"))?;
            Some(json!(elements))
        } else {
            None
        };
        let mut resource_patches = Vec::new();
        if let Some(title) = title {
            let expected = str_arg(&args, "expected_metadata_revision").ok_or_else(|| {
                anyhow::anyhow!("expected_metadata_revision required; read the template first")
            })?;
            let state = wp
                .get(&format!(
                    "mcp-for-page-builders/v1/resources/post_metadata/{id}"
                ))
                .await?;
            resource_patches.push(json!({"kind":"post_metadata","id":id.to_string(),"expected_revision":expected,"after":{"title":title,"status":state["value"]["status"]}}));
        }
        ApplyElementorPlan.run(json!({"plan":{"site_url":wp.base_url(),"page_id":id,"expected_revision":expected_revision,"operation_id":operation_id,"document_patch":document_patch,"resource_patches":resource_patches}}), wp).await
    }
}
