use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::str_arg;
use crate::elementor::ElementorService;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct ElementorRuntime;

pub struct InspectElementorCapabilities;

#[async_trait]
impl Tool for ElementorRuntime {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "elementor_runtime",
            description: "Inspect authenticated Elementor capabilities, installed versions, registered REST routes, active kit settings, and live widget or style schemas through the MCP bridge.",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["capabilities", "runtime", "style_schema", "widget_schema"], "default": "capabilities" },
                    "widget_type": { "type": "string", "description": "Required for widget_schema" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let service = ElementorService::new(wp);
        let action = str_arg(&args, "action").unwrap_or_else(|| "capabilities".into());
        let value = match action.as_str() {
            "capabilities" => service.capabilities().await?,
            "runtime" => service.runtime().await?,
            "style_schema" => service.style_schema().await?,
            "widget_schema" => {
                let widget_type = str_arg(&args, "widget_type")
                    .ok_or_else(|| anyhow::anyhow!("widget_type required for widget_schema"))?;
                service.element_schema(&widget_type).await?
            }
            _ => anyhow::bail!("Unknown Elementor runtime action: {action}"),
        };
        Ok(ToolResult::text(serde_json::to_string_pretty(&value)?))
    }
}

#[async_trait]
impl Tool for InspectElementorCapabilities {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "inspect_elementor_capabilities",
            description: "Inspect native Elementor abilities, permissions, availability, versions, routes, and live schemas.",
            input_schema: json!({"type":"object","properties":{}}),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let service = ElementorService::new(wp);
        let capabilities = service.capabilities().await?;
        let runtime = service.runtime().await?;
        let styles = service.style_schema().await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&json!({
            "capabilities": capabilities,
            "runtime": runtime,
            "style_schema": styles
        }))?))
    }
}
