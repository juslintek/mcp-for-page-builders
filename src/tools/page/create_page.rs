use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::{Tool, elementor_plan_tools::ApplyElementorPlan};
use crate::wp::WpClient;

pub struct CreatePage;

#[async_trait]
impl Tool for CreatePage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "create_page",
            description: "Create a page through the revision-checked native Elementor resource path.",
            input_schema: json!({"type":"object","required":["title","elementor_data"],"properties":{"title":{"type":"string"},"slug":{"type":"string"},"elementor_data":{"type":"string","description":"Full Elementor JSON array string"},"status":{"type":"string","enum":["publish","draft","private"],"default":"draft"},"operation_id":{"type":"string","description":"Stable operation ID; generated when omitted"}}}),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let title = str_arg(&args, "title").context("title required")?;
        let data = str_arg(&args, "elementor_data").context("elementor_data required")?;
        let document: Value =
            serde_json::from_str(&data).context("elementor_data is not valid JSON")?;
        let _: Vec<crate::types::Element> = serde_json::from_value(document.clone())
            .context("elementor_data must be a typed full Elementor tree")?;
        let status = str_arg(&args, "status").unwrap_or_else(|| "draft".into());
        if !["draft", "private", "publish"].contains(&status.as_str()) {
            anyhow::bail!("status must be draft, private, or publish");
        }
        let operation_id = str_arg(&args, "operation_id")
            .unwrap_or_else(|| format!("create_page_{}", crate::elementor::generate_id()));
        let mut after = json!({"title":title,"status":status,"document":document});
        if let Some(slug) = str_arg(&args, "slug") {
            after["slug"] = json!(slug);
        }
        let applied = ApplyElementorPlan.run(json!({"plan":{"site_url":wp.base_url(),"page_id":0,"expected_revision":"absent","operation_id":operation_id,"resource_patches":[{"kind":"page","id":"new","expected_revision":"absent","after":after}]}}), wp).await?;
        if applied.is_error == Some(true) {
            return Ok(applied);
        }
        let restore = format!(
            "page creation may already have occurred; inspect and restore with operation_id {operation_id} via mcp-for-page-builders/v1/resources/restore"
        );
        let receipt: Value = serde_json::from_str(
            applied
                .content
                .first()
                .and_then(|item| serde_json::to_value(item).ok())
                .and_then(|item| item["text"].as_str().map(str::to_owned))
                .as_deref()
                .context(restore.clone())?,
        )
        .with_context(|| restore.clone())?;
        let resources = receipt["resources"].as_array().context(restore.clone())?;
        anyhow::ensure!(
            receipt["status"] == "created"
                && resources.len() == 1
                && receipt["operation_id"].as_str() == Some(operation_id.as_str())
                && receipt["idempotent_replay"].is_boolean(),
            "{restore}"
        );
        let resource = &resources[0];
        let (id, canonical_id) = match &resource["id"] {
            Value::String(id) => (
                id.clone(),
                id.parse::<u64>()
                    .is_ok_and(|number| number > 0 && number.to_string() == *id),
            ),
            Value::Number(id) => (
                id.as_u64()
                    .map(|number| number.to_string())
                    .unwrap_or_default(),
                id.as_u64().is_some_and(|number| number > 0),
            ),
            _ => (String::new(), false),
        };
        let revision = resource["after_revision"].as_str().unwrap_or_default();
        anyhow::ensure!(
            resource["kind"] == "page"
                && canonical_id
                && revision.len() == 64
                && revision
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "{restore}"
        );
        let post = wp
            .get(&format!("wp/v2/pages/{id}"))
            .await
            .with_context(|| restore.clone())?;
        let mut resp = json!({"status":"created","id":id,"link":post["link"],"post_status":post["status"],"resource_revision":revision,"operation_id":receipt["operation_id"],"idempotent_replay":receipt["idempotent_replay"]});
        if !receipt["cache_cleared"].is_null() {
            resp["cache_cleared"] = receipt["cache_cleared"].clone();
        }
        Ok(ToolResult::text(serde_json::to_string(&resp)?))
    }
}
