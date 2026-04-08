use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;
use super::parse_conditions;

pub struct CreateTemplate;
#[async_trait]
impl Tool for CreateTemplate {
    fn def(&self) -> ToolDef {
        ToolDef { name: "create_template", description: "Create an Elementor template (header, footer, single, archive, page, popup, loop-item). When conditions are provided, the template is automatically activated site-wide via Theme Builder.",
            input_schema: json!({"type":"object","required":["title","template_type","elementor_data"],"properties":{"title":{"type":"string"},"template_type":{"type":"string","enum":["header","footer","single","single-post","single-page","archive","popup","loop-item","page","section"]},"elementor_data":{"type":"string"},"conditions":{"type":"array","items":{"type":"string"},"description":"Display conditions, e.g. ['include/general']."}}}) }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let title = str_arg(&args, "title").unwrap_or_default();
        let tpl_type = str_arg(&args, "template_type").unwrap_or_else(|| "page".into());
        let data = str_arg(&args, "elementor_data").unwrap_or_else(|| "[]".into());
        serde_json::from_str::<Value>(&data).map_err(|e| anyhow::anyhow!("Invalid elementor_data JSON: {e}"))?;
        let body = json!({"title": title, "status": "publish", "meta": {"_elementor_template_type": tpl_type, "_elementor_data": data, "_elementor_edit_mode": "builder"}});
        let result = wp.post("wp/v2/elementor_library", &body).await?;
        let id = result["id"].as_u64().unwrap_or(0);
        if let Some(conditions) = parse_conditions(&args) {
            ElementorService::new(wp).set_template_conditions(id, &tpl_type, &conditions).await?;
        } else {
            ElementorService::new(wp).clear_cache().await?;
        }
        Ok(ToolResult::text(format!("Created {tpl_type} template [{id}]: {title}")))
    }
}
