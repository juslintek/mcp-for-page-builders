use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct ListTemplates;
#[async_trait]
impl Tool for ListTemplates {
    fn def(&self) -> ToolDef {
        ToolDef { name: "list_templates", description: "List all Elementor templates with their types.",
            input_schema: json!({"type":"object","properties":{"template_type":{"type":"string"},"per_page":{"type":"integer","default":20}}}) }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let per_page = args.get("per_page").and_then(serde_json::Value::as_u64).unwrap_or(20);
        let result = wp.get(&format!("wp/v2/elementor_library?per_page={per_page}&status=any&context=edit")).await?;
        let items = result.as_array().ok_or_else(|| anyhow::anyhow!("Unexpected response"))?;
        let filter_type = str_arg(&args, "template_type");
        let mut lines = Vec::new();
        for item in items {
            let id = item["id"].as_u64().unwrap_or(0);
            let title = item["title"]["rendered"].as_str().unwrap_or("(no title)");
            let ttype = item["meta"]["_elementor_template_type"].as_str().unwrap_or("unknown");
            if let Some(ref ft) = filter_type && ttype != ft.as_str() { continue; }
            lines.push(format!("  [{id}] {title} ({ttype})"));
        }
        Ok(ToolResult::text(format!("{} templates:\n{}", lines.len(), lines.join("\n"))))
    }
}
