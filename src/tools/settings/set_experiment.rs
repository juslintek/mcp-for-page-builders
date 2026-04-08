use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct SetExperiment;

#[async_trait]
impl Tool for SetExperiment {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "set_experiment", description: "Enable or disable an Elementor experiment (feature flag).",
            input_schema: json!({
                "type": "object", "required": ["name", "state"],
                "properties": {
                    "name": { "type": "string", "description": "Experiment name (e.g. 'e_flexbox_positioning', 'container')" },
                    "state": { "type": "string", "enum": ["active", "inactive", "default"] }
                }
            }),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let name = str_arg(&args, "name").ok_or_else(|| anyhow::anyhow!("name required"))?;
        let state = str_arg(&args, "state").ok_or_else(|| anyhow::anyhow!("state required"))?;
        let body = json!({ "experiments": { &name: &state } });
        wp.post("elementor/v1/settings", &body).await?;
        Ok(ToolResult::text(format!("Set experiment '{name}' to '{state}'")))
    }
}
