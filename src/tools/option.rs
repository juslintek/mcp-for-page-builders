use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

pub struct GetWpOption;
#[async_trait]
impl Tool for GetWpOption {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_wp_option",
            description: "Read a WordPress option value. Uses the elementor-mcp/v1/option/ REST endpoint (requires the companion mu-plugin). Falls back to wp/v2/settings for standard options.",
            input_schema: json!({
                "type": "object", "required": ["name"],
                "properties": {
                    "name": {"type": "string", "description": "Option name, e.g. 'elementor_pro_theme_builder_conditions', 'blogname'"}
                }
            }),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let name = str_arg(&args, "name").ok_or_else(|| anyhow::anyhow!("name required"))?;

        // Try custom endpoint first
        match wp.get(&format!("elementor-mcp/v1/option/{name}")).await {
            Ok(val) => return Ok(ToolResult::text(serde_json::to_string_pretty(&val)?)),
            Err(_) => {}
        }

        // Fallback: try wp/v2/settings (only works for options registered with show_in_rest)
        let settings = wp.get("wp/v2/settings").await?;
        if let Some(val) = settings.get(&name) {
            return Ok(ToolResult::text(serde_json::to_string_pretty(val)?));
        }

        anyhow::bail!("Option '{name}' not found. The elementor-mcp/v1/option/ endpoint may not be available — install the companion mu-plugin or use WP-CLI.")
    }
}

pub struct SetWpOption;
#[async_trait]
impl Tool for SetWpOption {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "set_wp_option",
            description: "Write a WordPress option value. Uses the elementor-mcp/v1/option/ REST endpoint (requires the companion mu-plugin).",
            input_schema: json!({
                "type": "object", "required": ["name", "value"],
                "properties": {
                    "name": {"type": "string", "description": "Option name"},
                    "value": {"description": "Option value (string, number, object, or array)"}
                }
            }),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let name = str_arg(&args, "name").ok_or_else(|| anyhow::anyhow!("name required"))?;
        let value = args.get("value").ok_or_else(|| anyhow::anyhow!("value required"))?;

        wp.post(&format!("elementor-mcp/v1/option/{name}"), value).await?;
        Ok(ToolResult::text(format!("Option '{name}' updated")))
    }
}
