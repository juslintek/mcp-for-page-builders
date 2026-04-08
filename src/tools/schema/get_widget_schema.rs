use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;
use super::registry::SchemaRegistry;
use super::widget_schema::{COMMON_SETTINGS, TYPOGRAPHY_KEYS, COMMON_ALIASES};

pub struct GetWidgetSchema;

#[async_trait]
impl Tool for GetWidgetSchema {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "get_widget_schema",
            description: "Get the full schema for a widget type — all valid settings, common mistakes, and their corrections.",
            input_schema: json!({"type":"object","required":["widget_type"],"properties":{"widget_type":{"type":"string","description":"e.g. 'heading', 'text-editor', 'button'"}}}),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let wt = str_arg(&args, "widget_type").ok_or_else(|| anyhow::anyhow!("widget_type required"))?;
        let reg = SchemaRegistry::global();

        if let Some(schema) = reg.get(&wt) {
            let valid = reg.valid_keys(schema);
            let aliases: HashMap<&str, &str> = schema.aliases.iter()
                .chain(COMMON_ALIASES.iter())
                .map(|(k, v)| (*k, *v))
                .collect();

            let result = json!({
                "widget_type": schema.widget_type,
                "category": schema.category,
                "widget_settings": schema.settings,
                "common_settings": COMMON_SETTINGS,
                "typography_settings": TYPOGRAPHY_KEYS,
                "all_valid_keys": valid,
                "common_mistakes": aliases,
            });
            Ok(ToolResult::text(serde_json::to_string_pretty(&result)?))
        } else {
            let known: Vec<&str> = reg.all().iter().map(|s| s.widget_type).collect();
            Ok(ToolResult::error(format!("Unknown widget type '{wt}'. Known types: {}", known.join(", "))))
        }
    }
}
