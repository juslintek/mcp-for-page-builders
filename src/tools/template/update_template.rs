use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;
use super::parse_conditions;

pub struct UpdateTemplate;
#[async_trait]
impl Tool for UpdateTemplate {
    fn def(&self) -> ToolDef {
        ToolDef { name: "update_template", description: "Update an existing Elementor template. Can replace elementor_data, title, and/or conditions.",
            input_schema: json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"},"title":{"type":"string"},"elementor_data":{"type":"string"},"conditions":{"type":"array","items":{"type":"string"}}}}) }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let mut body = json!({});
        let mut meta = json!({});
        if let Some(title) = str_arg(&args, "title") { body["title"] = Value::String(title); }
        if let Some(data) = str_arg(&args, "elementor_data") {
            serde_json::from_str::<Value>(&data).map_err(|e| anyhow::anyhow!("Invalid elementor_data JSON: {e}"))?;
            meta["_elementor_data"] = Value::String(data);
        }
        if meta.as_object().is_some_and(|m| !m.is_empty()) { body["meta"] = meta; }
        if body.as_object().is_none_or(serde_json::Map::is_empty) && parse_conditions(&args).is_none() {
            anyhow::bail!("Nothing to update — provide title, elementor_data, or conditions");
        }
        if body.as_object().is_some_and(|m| !m.is_empty()) {
            let jid = wp.session.as_ref().map(|s| s.record("update_template", wp.base_url(), &format!("template:{id}")));
            wp.post(&format!("wp/v2/elementor_library/{id}"), &body).await?;
            if let (Some(s), Some(jid)) = (&wp.session, jid) { s.complete(&jid); }
        }
        if let Some(conditions) = parse_conditions(&args) {
            let tpl = wp.get(&format!("wp/v2/elementor_library/{id}?context=edit")).await?;
            let tpl_type = tpl["meta"]["_elementor_template_type"].as_str().unwrap_or("page").to_string();
            ElementorService::new(wp).set_template_conditions(id, &tpl_type, &conditions).await?;
        } else {
            ElementorService::new(wp).clear_cache().await?;
        }
        Ok(ToolResult::text(format!("Updated template [{id}]")))
    }
}
