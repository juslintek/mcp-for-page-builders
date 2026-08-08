use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::types::{Tool, ToolDef, ToolResult};
use crate::wp::WpClient;

pub struct Wp7McpCapabilities;
pub struct Wp7TemplateParts;
pub struct Wp7BlockRender;

#[async_trait]
impl Tool for Wp7McpCapabilities {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "wp7_mcp_capabilities",
            description: "Query native WordPress 7 MCP capabilities, REST controller endpoints (/wp-json/mcp/v1/), block renderer, and Application Passwords 2.0 scope status.",
            input_schema: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        wp.require_configured()?;

        let mcp_res = wp.get("mcp/v1/capabilities").await.ok();
        let bridge_res = wp.get("mcp-for-page-builders/v1/status").await.ok();
        let fse_res = wp.get("wp/v2/templates").await.ok();

        let capabilities = json!({
            "wp_7_mcp_controller": mcp_res.is_some(),
            "mcp_v1_capabilities": mcp_res.unwrap_or_else(|| json!({"status": "unavailable", "hint": "MCP bridge plugin provides native fallback"})),
            "bridge_status": bridge_res.unwrap_or_else(|| json!({"status": "not_installed"})),
            "fse_templates_available": fse_res.is_some(),
            "features": [
                "mcp_json_rpc_over_rest",
                "application_passwords_2_scopes",
                "fse_block_templates",
                "gutenberg_block_renderer"
            ]
        });

        Ok(ToolResult::text(serde_json::to_string_pretty(&capabilities)?))
    }
}

#[async_trait]
impl Tool for Wp7TemplateParts {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "wp7_template_parts",
            description: "List or retrieve WordPress 7 Full Site Editing (FSE) block templates and template parts (wp/v2/templates & wp/v2/template-parts).",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "type": { "type": "string", "enum": ["templates", "template-parts"], "default": "templates" },
                    "slug": { "type": "string", "description": "Specific template slug (e.g. 'header', 'footer', 'single')" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        wp.require_configured()?;
        let ttype = args.get("type").and_then(Value::as_str).unwrap_or("templates");
        let endpoint = if let Some(s) = args.get("slug").and_then(Value::as_str) {
            format!("wp/v2/{ttype}/{s}")
        } else {
            format!("wp/v2/{ttype}")
        };

        let result = wp.get(&endpoint).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
    }
}

#[async_trait]
impl Tool for Wp7BlockRender {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "wp7_block_render",
            description: "Render a Gutenberg / WordPress 7 block by name and attributes using the native REST block renderer (wp/v2/block-renderer/{name}).",
            input_schema: json!({
                "type": "object",
                "required": ["block_name"],
                "properties": {
                    "block_name": { "type": "string", "description": "Block name (e.g. 'core/paragraph', 'core/heading', 'core/columns')" },
                    "attributes": { "type": "object", "description": "Block attributes dictionary" },
                    "post_id": { "type": "integer", "description": "Optional context post ID" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        wp.require_configured()?;
        let block_name = args["block_name"].as_str().ok_or_else(|| anyhow::anyhow!("block_name required"))?;
        let mut query = json!({});
        if let Some(attrs) = args.get("attributes") {
            query["attributes"] = json!(serde_json::to_string(attrs)?);
        }
        if let Some(post_id) = args.get("post_id").and_then(Value::as_i64) {
            query["post_id"] = json!(post_id);
        }

        let endpoint = format!("wp/v2/block-renderer/{block_name}");
        let result = wp.request("GET", &endpoint, None, Some(&query)).await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
    }
}
