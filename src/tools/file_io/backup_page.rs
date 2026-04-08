use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::Path;

use crate::args::{str_arg, u64_arg};
use crate::mcp::{ToolDef, ToolResult};
use crate::wp::WpClient;
use crate::tools::Tool;

pub struct BackupPage;

#[async_trait]
impl Tool for BackupPage {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "backup_page",
            description: "Backup a page's current Elementor data to a timestamped file before making changes.",
            input_schema: json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"},"dir":{"type":"string","description":"Backup directory (default: /tmp)"}}}),
        }
    }

    async fn run(&self, args: Value, wp: &WpClient) -> Result<ToolResult> {
        let id = u64_arg(&args, "id").ok_or_else(|| anyhow::anyhow!("id required"))?;
        let dir = str_arg(&args, "dir").unwrap_or_else(|| "/tmp".into());

        let page = wp.get(&format!("wp/v2/pages/{id}?context=edit")).await?;
        let data = page.get("meta").and_then(|m| m.get("_elementor_data"))
            .ok_or_else(|| anyhow::anyhow!("No _elementor_data found on page {id}"))?;

        let content = match data.as_str() {
            Some(s) => { let parsed: Value = serde_json::from_str(s)?; serde_json::to_string_pretty(&parsed)? }
            None => serde_json::to_string_pretty(data)?,
        };

        let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
        let full_path = Path::new(&dir).join(format!("elementor-backup-{id}-{timestamp}.json"));
        tokio::fs::write(&full_path, &content).await?;
        Ok(ToolResult::text(format!("Backed up page {id} to {}", full_path.display())))
    }
}
