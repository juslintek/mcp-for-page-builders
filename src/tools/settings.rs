use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)?.as_str().map(|s| s.to_string())
}

// ── GetKitSchema ──────────────────────────────────────────────────────────────

pub struct GetKitSchema;

#[async_trait]
impl Tool for GetKitSchema {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_kit_schema",
            description: "Get the Elementor kit schema — all available kit settings, their types, and defaults. Useful for discovering what can be configured.",
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let schema = wp.get("angie/v1/elementor-kit/schema").await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&schema)?))
    }
}

// ── GetKitDefaults ────────────────────────────────────────────────────────────

pub struct GetKitDefaults;

#[async_trait]
impl Tool for GetKitDefaults {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_kit_defaults",
            description: "Get Elementor kit element defaults — the default settings applied to all widgets of each type.",
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let defaults = wp.get("elementor/v1/kit-elements-defaults").await?;
        Ok(ToolResult::text(serde_json::to_string_pretty(&defaults)?))
    }
}

// ── GetExperiments ────────────────────────────────────────────────────────────

pub struct GetExperiments;

#[async_trait]
impl Tool for GetExperiments {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_experiments",
            description: "Get all Elementor experiments (feature flags) and their current state.",
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        // Experiments are embedded in the settings page — fetch via admin-ajax or settings endpoint
        let settings = wp.get("elementor/v1/settings").await?;
        let experiments = settings.get("experiments").cloned().unwrap_or(json!({}));
        Ok(ToolResult::text(serde_json::to_string_pretty(&experiments)?))
    }
}

// ── SetExperiment ─────────────────────────────────────────────────────────────

pub struct SetExperiment;

#[async_trait]
impl Tool for SetExperiment {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "set_experiment",
            description: "Enable or disable an Elementor experiment (feature flag).",
            input_schema: json!({
                "type": "object",
                "required": ["name", "state"],
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
