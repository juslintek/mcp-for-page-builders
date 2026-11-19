use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::args::str_arg;
use crate::mcp::{ToolDef, ToolResult};
use crate::tools::Tool;
use crate::wp::WpClient;

pub struct SetExperiment;

#[async_trait]
impl Tool for SetExperiment {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "set_experiment",
            description: "Unsupported until a checked native adapter is available; no mutation is made.",
            input_schema: json!({
                "type": "object", "required": ["name", "state"],
                "properties": {
                    "name": { "type": "string", "description": "Experiment name (e.g. 'e_flexbox_positioning', 'container')" },
                    "state": { "type": "string", "enum": ["active", "inactive", "default"] }
                }
            }),
        }
    }
    async fn run(&self, args: Value, _wp: &WpClient) -> Result<ToolResult> {
        let name = str_arg(&args, "name").ok_or_else(|| anyhow::anyhow!("name required"))?;
        let state = str_arg(&args, "state").ok_or_else(|| anyhow::anyhow!("state required"))?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            anyhow::bail!(
                "experiment name must contain only letters, digits, underscores, or hyphens; no mutation was made"
            );
        }
        if !matches!(state.as_str(), "active" | "inactive" | "default") {
            anyhow::bail!("state must be active, inactive, or default; no mutation was made");
        }
        anyhow::bail!(
            "Elementor experiment writes are unsupported until a checked native adapter is available; no mutation was made"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::SetExperiment;
    use crate::{tools::Tool, wp::WpClient};
    use serde_json::json;

    #[tokio::test]
    async fn valid_experiment_write_fails_before_network_without_native_adapter() {
        let wp = WpClient::new("http://127.0.0.1:1", "user", "password");
        let error = match SetExperiment
            .run(json!({"name":"container","state":"active"}), &wp)
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("unsupported native writes must fail closed"),
        };
        assert!(
            error
                .to_string()
                .contains("unsupported until a checked native adapter")
        );
        assert!(error.to_string().contains("no mutation was made"));
    }
}
