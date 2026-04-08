use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct SetGlobalTypography;

#[async_trait]
impl Tool for SetGlobalTypography {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "set_global_typography",
            description: "Create or update a global typography preset.",
            input_schema: json!({
                "type": "object", "required": ["id", "title"],
                "properties": {
                    "id": { "type": "string" }, "title": { "type": "string" },
                    "font_family": { "type": "string" },
                    "font_size": { "type": "object" }, "font_weight": { "type": "string" },
                    "line_height": { "type": "object" }, "letter_spacing": { "type": "object" }
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
                let ekey = format!("typography_{key}");
                value[ekey] = v.clone();
            }
        }

        let body = json!({ "id": id, "title": title, "value": value });
        wp.post(&format!("elementor/v1/globals/typography/{id}"), &body).await?;
        Ok(ToolResult::text(format!("Set global typography '{id}': {title}")))
    }
}
