use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;
use super::registry::SchemaRegistry;
use super::widget_schema::COMMON_SETTINGS;

pub struct ListWidgets;

#[async_trait]
impl Tool for ListWidgets {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "list_widgets",
            description: "List all known widget types with their categories. These are the widgets with bundled schema validation.",
            input_schema: json!({ "type": "object", "properties": {} }),
        }
    }

    async fn run(&self, _args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let schemas = SchemaRegistry::global().all();
        let mut lines = vec![format!("{} widget schemas available:", schemas.len())];
        for s in schemas {
            lines.push(format!("  {} [{}] — {} settings", s.widget_type, s.category, s.settings.len()));
        }
        lines.push(String::new());
        lines.push(format!("Plus {} common settings shared by all widgets.", COMMON_SETTINGS.len()));
        Ok(ToolResult::text(lines.join("\n")))
    }
}
