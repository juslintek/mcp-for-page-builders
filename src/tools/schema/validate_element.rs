use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;
use super::registry::SchemaRegistry;

pub struct ValidateElement;

#[async_trait]
impl Tool for ValidateElement {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "validate_element",
            description: "Validate an element's JSON against its widget schema. Reports invalid settings with 'did you mean?' suggestions.\n\nWorkflow: Use before add_element or update_element to catch setting key mistakes early.",
            input_schema: json!({"type":"object","required":["element"],"properties":{"element":{"type":"object","description":"Full element JSON to validate"}}}),
        }
    }

    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let el = args.get("element").ok_or_else(|| anyhow::anyhow!("element required"))?;
        let el_type = el.get("elType").and_then(|v| v.as_str()).unwrap_or("");
        let widget_type = el.get("widgetType").and_then(|v| v.as_str());

        let mut errors: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        if el.get("id").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
            warnings.push("Missing 'id' — will be auto-generated on add_element".into());
        }
        if el_type.is_empty() {
            errors.push("Missing 'elType' — must be 'widget', 'container', 'section', or 'column'".into());
        }

        if el_type == "widget" {
            let Some(wt) = widget_type else { errors.push("Widget element missing 'widgetType'".into()); return Ok(format_validation(&errors, &warnings)); };
            let reg = SchemaRegistry::global();
            match reg.get(wt) {
                Some(schema) => {
                    let valid = reg.valid_keys(schema);
                    if let Some(settings) = el.get("settings").and_then(|s| s.as_object()) {
                        for key in settings.keys() {
                            if !valid.iter().any(|v| v == key) {
                                match reg.suggest_fix(key, schema) {
                                    Some(suggestion) => errors.push(format!("Invalid setting '{key}' on widget '{wt}' — did you mean '{suggestion}'?")),
                                    None => warnings.push(format!("Unknown setting '{key}' on widget '{wt}' — not in bundled schema (may be valid for addons)")),
                                }
                            }
                        }
                    }
                }
                None => { warnings.push(format!("Widget type '{wt}' not in bundled schema — settings not validated")); }
            }
        }

        if errors.is_empty() && warnings.is_empty() {
            Ok(ToolResult::text("✓ Element is valid."))
        } else {
            Ok(format_validation(&errors, &warnings))
        }
    }
}

fn format_validation(errors: &[String], warnings: &[String]) -> ToolResult {
    let mut lines = Vec::new();
    if !errors.is_empty() {
        lines.push(format!("✗ {} error(s):", errors.len()));
        for e in errors { lines.push(format!("  ERROR: {e}")); }
    }
    if !warnings.is_empty() {
        lines.push(format!("⚠ {} warning(s):", warnings.len()));
        for w in warnings { lines.push(format!("  WARN: {w}")); }
    }
    if errors.is_empty() { ToolResult::text(lines.join("\n")) } else { ToolResult::error(lines.join("\n")) }
}
