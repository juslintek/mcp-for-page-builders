use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::{str_arg, u64_arg};
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

fn parse_conditions(args: &Value) -> Option<Vec<String>> {
    args.get("conditions")?.as_array().map(|arr| {
        arr.iter().filter_map(|v| v.as_str().map(String::from)).collect()
    })
}

pub struct CreateTemplate;
#[async_trait]
impl Tool for CreateTemplate {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "create_template",
            description: "Create an Elementor template (header, footer, single, archive, page, popup, loop-item). When conditions are provided, the template is automatically activated site-wide via Theme Builder.",
            input_schema: json!({
                "type": "object", "required": ["title", "template_type", "elementor_data"],
                "properties": {
                    "title": {"type": "string"},
                    "template_type": {"type": "string", "enum": ["header","footer","single","single-post","single-page","archive","popup","loop-item","page","section"]},
                    "elementor_data": {"type": "string", "description": "Elementor JSON array string"},
                    "conditions": {"type": "array", "items": {"type": "string"}, "description": "Display conditions, e.g. ['include/general']. Sets both post meta AND global Theme Builder option."}
                }
            }),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let title = str_arg(&args, "title").unwrap_or_default();
        let tpl_type = str_arg(&args, "template_type").unwrap_or_else(|| "page".into());
        let data = str_arg(&args, "elementor_data").unwrap_or_else(|| "[]".into());

        serde_json::from_str::<Value>(&data)
            .map_err(|e| anyhow::anyhow!("Invalid elementor_data JSON: {e}"))?;

        let body = json!({
            "title": title, "status": "publish",
            "meta": {
                "_elementor_template_type": tpl_type,
                "_elementor_data": data,
                "_elementor_edit_mode": "builder"
            }
        });
        let result = wp.post("wp/v2/elementor_library", &body).await?;
        let id = result["id"].as_u64().unwrap_or(0);

        // Set display conditions (post meta + global option + cache clear)
        if let Some(conditions) = parse_conditions(&args) {
            ElementorService::new(wp).set_template_conditions(id, &tpl_type, &conditions).await?;
        } else {
            ElementorService::new(wp).clear_cache().await?;
        }

        Ok(ToolResult::text(format!("Created {tpl_type} template [{id}]: {title}")))
    }
}

pub struct UpdateTemplate;
#[async_trait]
impl Tool for UpdateTemplate {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "update_template",
            description: "Update an existing Elementor template. Can replace elementor_data, title, and/or conditions.",
            input_schema: json!({
                "type": "object", "required": ["id"],
                "properties": {
                    "id": {"type": "integer"},
                    "title": {"type": "string"},
                    "elementor_data": {"type": "string", "description": "New Elementor JSON array string"},
                    "conditions": {"type": "array", "items": {"type": "string"}, "description": "Display conditions, e.g. ['include/general']"}
                }
            }),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;

        let mut body = json!({});
        let mut meta = json!({});

        if let Some(title) = str_arg(&args, "title") {
            body["title"] = Value::String(title);
        }
        if let Some(data) = str_arg(&args, "elementor_data") {
            serde_json::from_str::<Value>(&data)
                .map_err(|e| anyhow::anyhow!("Invalid elementor_data JSON: {e}"))?;
            meta["_elementor_data"] = Value::String(data);
        }
        if meta.as_object().map_or(false, |m| !m.is_empty()) {
            body["meta"] = meta;
        }

        if body.as_object().map_or(true, |m| m.is_empty()) && parse_conditions(&args).is_none() {
            anyhow::bail!("Nothing to update — provide title, elementor_data, or conditions");
        }

        if body.as_object().map_or(false, |m| !m.is_empty()) {
            wp.post(&format!("wp/v2/elementor_library/{id}"), &body).await?;
        }

        // Update conditions if provided
        if let Some(conditions) = parse_conditions(&args) {
            // Read template type from existing template
            let tpl = wp.get(&format!("wp/v2/elementor_library/{id}?context=edit")).await?;
            let tpl_type = tpl["meta"]["_elementor_template_type"]
                .as_str().unwrap_or("page").to_string();
            ElementorService::new(wp).set_template_conditions(id, &tpl_type, &conditions).await?;
        } else {
            ElementorService::new(wp).clear_cache().await?;
        }

        Ok(ToolResult::text(format!("Updated template [{id}]")))
    }
}

pub struct ListTemplates;
#[async_trait]
impl Tool for ListTemplates {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_templates",
            description: "List all Elementor templates with their types.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "template_type": {"type": "string", "description": "Filter by type (header, footer, etc.)"},
                    "per_page": {"type": "integer", "default": 20}
                }
            }),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let per_page = args.get("per_page").and_then(|v| v.as_u64()).unwrap_or(20);
        let result = wp.get(&format!("wp/v2/elementor_library?per_page={per_page}&status=any&context=edit")).await?;
        let items = result.as_array().ok_or_else(|| anyhow::anyhow!("Unexpected response"))?;

        let filter_type = str_arg(&args, "template_type");
        let mut lines = Vec::new();
        for item in items {
            let id = item["id"].as_u64().unwrap_or(0);
            let title = item["title"]["rendered"].as_str().unwrap_or("(no title)");
            let ttype = item["meta"]["_elementor_template_type"].as_str().unwrap_or("unknown");
            if let Some(ref ft) = filter_type {
                if ttype != ft.as_str() { continue; }
            }
            lines.push(format!("  [{id}] {title} ({ttype})"));
        }
        Ok(ToolResult::text(format!("{} templates:\n{}", lines.len(), lines.join("\n"))))
    }
}

pub struct GetTemplate;
#[async_trait]
impl Tool for GetTemplate {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_template",
            description: "Get an Elementor template by ID including its data.",
            input_schema: json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"}}}),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let tpl = wp.get(&format!("wp/v2/elementor_library/{id}?context=edit")).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&tpl)?))
    }
}

pub struct DeleteTemplate;
#[async_trait]
impl Tool for DeleteTemplate {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "delete_template",
            description: "Delete an Elementor template.",
            input_schema: json!({
                "type":"object","required":["id"],
                "properties":{"id":{"type":"integer"},"force":{"type":"boolean","default":false}}
            }),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
        let path = if force { format!("wp/v2/elementor_library/{id}?force=true") } else { format!("wp/v2/elementor_library/{id}") };
        wp.delete(&path).await?;
        Ok(ToolResult::text(format!("Deleted template {id}")))
    }
}
