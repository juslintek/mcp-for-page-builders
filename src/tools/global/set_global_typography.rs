use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct SetGlobalTypography;

#[async_trait]
impl Tool for SetGlobalTypography {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "set_global_typography",
            description: "Create or update a global typography preset.",
            input_schema: json!({
                "type": "object", "required": ["id", "title", "expected_revision"],
                "properties": {
                    "id": { "type": "string" }, "title": { "type": "string" },
                    "font_family": { "type": "string" },
                    "font_size": { "type": "object" }, "font_weight": { "type": "string" },
                    "line_height": { "type": "object" }, "letter_spacing": { "type": "object" },
                    "expected_revision": { "type": "string", "description": "Revision returned by get_global_typography" }
                }
            }),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = str_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let title = str_arg(&args, "title").ok_or_else(|| anyhow::anyhow!("title required"))?;
        let revision = str_arg(&args, "expected_revision").ok_or_else(|| {
            anyhow::anyhow!("expected_revision required; read global typography first")
        })?;

        let mut value = json!({});
        for key in &[
            "font_family",
            "font_size",
            "font_weight",
            "line_height",
            "letter_spacing",
        ] {
            if let Some(v) = args.get(*key) {
                let ekey = format!("typography_{key}");
                value[ekey] = v.clone();
            }
        }

        let mut settings = super::resource::kit_settings(wp).await?;
        let collection =
            super::resource::collection(&settings, "system_typography", "custom_typography", &id);
        let mut preset = value.as_object().cloned().unwrap_or_default();
        preset.insert("title".into(), Value::String(title.clone()));
        super::resource::upsert(&mut settings, &collection, &id, Value::Object(preset))?;
        super::resource::save_kit_settings(wp, settings, &revision).await?;
        Ok(ToolResult::text(format!(
            "Set global typography '{id}': {title}"
        )))
    }
}
