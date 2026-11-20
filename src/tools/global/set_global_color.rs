use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct SetGlobalColor;

#[async_trait]
impl Tool for SetGlobalColor {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "set_global_color",
            description: "Create or update a global color. Use the color ID to update existing.",
            input_schema: json!({
                "type": "object",
                "required": ["id", "title", "color", "expected_revision"],
                "properties": {
                    "id": { "type": "string", "description": "Color ID (e.g. 'primary', 'secondary', or custom ID)" },
                    "title": { "type": "string", "description": "Display name" },
                    "color": { "type": "string", "description": "Hex color value (e.g. '#0C91BA')" },
                    "expected_revision": { "type": "string", "description": "Revision returned by get_global_colors" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = str_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let title = str_arg(&args, "title").ok_or_else(|| anyhow::anyhow!("title required"))?;
        let color = str_arg(&args, "color").ok_or_else(|| anyhow::anyhow!("color required"))?;
        let revision = str_arg(&args, "expected_revision").ok_or_else(|| {
            anyhow::anyhow!("expected_revision required; read global colors first")
        })?;
        let mut settings = super::resource::kit_settings(wp).await?;
        let collection =
            super::resource::collection(&settings, "system_colors", "custom_colors", &id);
        super::resource::upsert(
            &mut settings,
            &collection,
            &id,
            json!({"title":title,"color":color}),
        )?;
        let cache = super::resource::save_kit_settings(wp, settings, &revision).await?;
        Ok(ToolResult::text(format!(
            "Set global color '{id}': {title} = {color}{}",
            cache.summary_line()
        )))
    }
}
