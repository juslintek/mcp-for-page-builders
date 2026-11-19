use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct SetWpOption;

fn validate_option_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        anyhow::bail!("name must contain only letters, digits, underscores, or hyphens");
    }

    let lower = name.to_ascii_lowercase();
    if [
        "elementor_",
        "_elementor_",
        "elementor_experiment-",
        "_mcp_elementor_",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
    {
        anyhow::bail!(
            "Elementor native and bridge options cannot be written through set_wp_option"
        );
    }
    Ok(())
}

fn option_payload(value: &Value) -> Value {
    json!({"value": value})
}

#[async_trait]
impl Tool for SetWpOption {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "set_wp_option",
            description: "Write a WordPress option value. Uses the mcp-for-page-builders/v1/option/ REST endpoint (requires the companion mu-plugin).",
            input_schema: json!({"type":"object","required":["name","value"],"properties":{"name":{"type":"string"},"value":{"description":"Option value (string, number, object, or array)"}}}),
        }
    }
    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let name = str_arg(&args, "name").ok_or_else(|| anyhow::anyhow!("name required"))?;
        validate_option_name(&name)?;
        let value = args
            .get("value")
            .ok_or_else(|| anyhow::anyhow!("value required"))?;
        let jid = wp
            .session
            .as_ref()
            .map(|s| s.record("set_wp_option", wp.base_url(), &format!("option:{name}")));
        wp.post(
            &format!("mcp-for-page-builders/v1/option/{name}"),
            &option_payload(value),
        )
        .await?;
        if let (Some(s), Some(id)) = (&wp.session, jid) {
            s.complete(&id);
        }
        Ok(ToolResult::text(format!("Option '{name}' updated")))
    }
}

#[cfg(test)]
mod tests {
    use super::{option_payload, validate_option_name};
    use serde_json::json;

    #[test]
    fn wraps_values_and_rejects_elementor_and_internal_options() {
        assert_eq!(
            option_payload(&json!([1, {"ok": true}])),
            json!({"value": [1, {"ok": true}]})
        );
        for name in [
            "elementor_active",
            "_elementor_data",
            "elementor_experiment-e_atomic",
            "_mcp_elementor_editor_lease_1",
            "ELEMENTOR_active",
        ] {
            assert!(
                validate_option_name(name).is_err(),
                "accepted protected option"
            );
        }
        assert!(validate_option_name("ordinary-option_1").is_ok());
        assert!(validate_option_name("bad/name").is_err());
    }
}
