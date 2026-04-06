use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use super::Tool;

// ── GetGlobalColors ───────────────────────────────────────────────────────────

pub struct GetGlobalColors;

#[async_trait]
impl Tool for GetGlobalColors {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_global_colors",
            description: "Get all Elementor global colors (design tokens).",
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let globals = wp.get("elementor/v1/globals").await?;
        let colors = globals.get("colors").cloned().unwrap_or(json!({}));
        Ok(ToolResult::text(serde_json::to_string_pretty(&colors)?))
    }
}

// ── SetGlobalColor ────────────────────────────────────────────────────────────

pub struct SetGlobalColor;

#[async_trait]
impl Tool for SetGlobalColor {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "set_global_color",
            description: "Create or update a global color. Use the color ID to update existing.",
            input_schema: json!({
                "type": "object",
                "required": ["id", "title", "color"],
                "properties": {
                    "id": { "type": "string", "description": "Color ID (e.g. 'primary', 'secondary', or custom ID)" },
                    "title": { "type": "string", "description": "Display name" },
                    "color": { "type": "string", "description": "Hex color value (e.g. '#0C91BA')" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = str_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let title = str_arg(&args, "title").ok_or_else(|| anyhow::anyhow!("title required"))?;
        let color = str_arg(&args, "color").ok_or_else(|| anyhow::anyhow!("color required"))?;

        let body = json!({ "id": id, "title": title, "value": color });
        wp.post(&format!("elementor/v1/globals/colors/{id}"), &body).await?;
        Ok(ToolResult::text(format!("Set global color '{id}': {title} = {color}")))
    }
}

// ── DeleteGlobalColor ─────────────────────────────────────────────────────────

pub struct DeleteGlobalColor;

#[async_trait]
impl Tool for DeleteGlobalColor {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "delete_global_color",
            description: "Delete a global color by ID.",
            input_schema: json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string", "description": "Color ID to delete" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = str_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        wp.delete(&format!("elementor/v1/globals/colors/{id}")).await?;
        Ok(ToolResult::text(format!("Deleted global color '{id}'")))
    }
}

// ── GetGlobalTypography ───────────────────────────────────────────────────────

pub struct GetGlobalTypography;

#[async_trait]
impl Tool for GetGlobalTypography {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_global_typography",
            description: "Get all Elementor global typography presets.",
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn run(&self, _args: Value, wp: &WpClient) -> Result<ToolResult> {
        let globals = wp.get("elementor/v1/globals").await?;
        let typo = globals.get("typography").cloned().unwrap_or(json!({}));
        Ok(ToolResult::text(serde_json::to_string_pretty(&typo)?))
    }
}

// ── SetGlobalTypography ───────────────────────────────────────────────────────

pub struct SetGlobalTypography;

#[async_trait]
impl Tool for SetGlobalTypography {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "set_global_typography",
            description: "Create or update a global typography preset.",
            input_schema: json!({
                "type": "object",
                "required": ["id", "title"],
                "properties": {
                    "id": { "type": "string", "description": "Typography ID" },
                    "title": { "type": "string", "description": "Display name" },
                    "font_family": { "type": "string" },
                    "font_size": { "type": "object", "description": "{\"unit\":\"px\",\"size\":16}" },
                    "font_weight": { "type": "string" },
                    "line_height": { "type": "object", "description": "{\"unit\":\"em\",\"size\":1.5}" },
                    "letter_spacing": { "type": "object" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = str_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let title = str_arg(&args, "title").ok_or_else(|| anyhow::anyhow!("title required"))?;

        let mut value = json!({});
        for key in &["font_family", "font_size", "font_weight", "line_height", "letter_spacing"] {
            if let Some(v) = args.get(*key) {
                // Elementor uses typography_ prefix in the value object
                let ekey = format!("typography_{key}");
                value[ekey] = v.clone();
            }
        }

        let body = json!({ "id": id, "title": title, "value": value });
        wp.post(&format!("elementor/v1/globals/typography/{id}"), &body).await?;
        Ok(ToolResult::text(format!("Set global typography '{id}': {title}")))
    }
}

// ── DeleteGlobalTypography ────────────────────────────────────────────────────

pub struct DeleteGlobalTypography;

#[async_trait]
impl Tool for DeleteGlobalTypography {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "delete_global_typography",
            description: "Delete a global typography preset by ID.",
            input_schema: json!({
                "type": "object",
                "required": ["id"],
                "properties": {
                    "id": { "type": "string" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = str_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        wp.delete(&format!("elementor/v1/globals/typography/{id}")).await?;
        Ok(ToolResult::text(format!("Deleted global typography '{id}'")))
    }
}
