use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::{Tool, elementor_plan_tools::ApplyElementorPlan};
use crate::wp::WpClient;

pub struct CreateTemplate;

#[async_trait]
impl Tool for CreateTemplate {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "create_template",
            description: "Create a revision-checked draft Elementor template. Conditions are not supported during creation.",
            input_schema: json!({"type":"object","required":["title","template_type","elementor_data"],"properties":{"title":{"type":"string"},"template_type":{"type":"string","enum":["header","footer","single","single-post","single-page","archive","popup","loop-item","page","section"]},"elementor_data":{"type":"string"},"operation_id":{"type":"string","description":"Stable operation ID; generated when omitted"},"conditions":{"type":"array","items":{"type":"string"},"description":"Unsupported until checked native conditions persistence is available"}}}),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        if args.get("conditions").is_some() {
            anyhow::bail!(
                "conditions are unsupported in checked template creation; no changes were made"
            );
        }
        let title = str_arg(&args, "title").context("title required")?;
        let template_type = str_arg(&args, "template_type").context("template_type required")?;
        let data = str_arg(&args, "elementor_data").context("elementor_data required")?;
        let document: Value =
            serde_json::from_str(&data).context("elementor_data is not valid JSON")?;
        let _: Vec<crate::types::Element> = serde_json::from_value(document.clone())
            .context("elementor_data must be a typed full Elementor tree")?;
        let operation_id = str_arg(&args, "operation_id")
            .unwrap_or_else(|| format!("create_template_{}", crate::elementor::generate_id()));
        let applied = ApplyElementorPlan.run(json!({"plan":{"site_url":wp.base_url(),"page_id":0,"expected_revision":"absent","operation_id":operation_id,"resource_patches":[{"kind":"template","id":"new","expected_revision":"absent","after":{"title":title,"status":"draft","type":template_type,"document":document}}]}}), wp).await?;
        if applied.is_error == Some(true) {
            return Ok(applied);
        }
        let receipt = creation_receipt(&applied, "template", &operation_id)?;
        let id = receipt["id"]
            .as_str()
            .context("validated template ID missing")?;
        Ok(ToolResult::text(serde_json::to_string(
            &json!({"status":"created","id":id,"template_type":template_type,"post_status":"draft","resource_revision":receipt["resource_revision"],"operation_id":receipt["operation_id"],"idempotent_replay":receipt["idempotent_replay"]}),
        )?))
    }
}

fn creation_receipt(applied: &ToolResult, kind: &str, operation_id: &str) -> Result<Value> {
    let failure = || {
        format!(
            "{kind} creation may already have occurred; inspect and restore with operation_id {operation_id} via mcp-for-page-builders/v1/resources/restore"
        )
    };
    let text = applied
        .content
        .first()
        .and_then(|item| serde_json::to_value(item).ok())
        .and_then(|item| item["text"].as_str().map(str::to_owned))
        .context(failure())?;
    let receipt: Value = serde_json::from_str(&text).with_context(failure)?;
    let resources = receipt["resources"].as_array().context(failure())?;
    if receipt["status"] != "created"
        || resources.len() != 1
        || receipt["operation_id"].as_str() != Some(operation_id)
        || !receipt["idempotent_replay"].is_boolean()
    {
        anyhow::bail!("{}", failure());
    }
    let resource = &resources[0];
    let (id, canonical_id) = match &resource["id"] {
        Value::String(id) => (
            id.clone(),
            id.parse::<u64>()
                .is_ok_and(|number| number > 0 && number.to_string() == *id),
        ),
        Value::Number(id) => (
            id.as_u64().map(|id| id.to_string()).unwrap_or_default(),
            id.as_u64().is_some_and(|id| id > 0),
        ),
        _ => (String::new(), false),
    };
    let revision = resource["after_revision"].as_str().unwrap_or_default();
    if resource["kind"].as_str() != Some(kind)
        || !canonical_id
        || revision.len() != 64
        || !revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        anyhow::bail!("{}", failure());
    }
    Ok(
        json!({"id":id,"resource_revision":revision,"operation_id":operation_id,"idempotent_replay":receipt["idempotent_replay"]}),
    )
}
